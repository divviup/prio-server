use chrono::NaiveDateTime;
use facilitator::{
    aggregation::BatchAggregator,
    batch::{Batch, BatchReader},
    idl::{InvalidPacket, Packet, SumPart},
    intake::BatchIntaker,
    sample::{generate_ingestion_sample, SampleOutput},
    test_utils::{
        default_facilitator_signing_private_key, default_facilitator_signing_public_key,
        default_ingestor_private_key, default_ingestor_public_key, default_pha_signing_private_key,
        default_pha_signing_public_key, log_init, DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY,
        DEFAULT_PHA_ECIES_PRIVATE_KEY,
    },
    transport::{
        LocalFileTransport, SignableTransport, VerifiableAndDecryptableTransport,
        VerifiableTransport,
    },
    Error,
};
use prio::{encrypt::PrivateKey, util::reconstruct_shares};
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn end_to_end() {
    end_to_end_test(None, None)
}

#[test]
fn inconsistent_ingestion_batches() {
    // Have sample generation drop every third and every fourth packet from the
    // PHA and facilitator ingestion batches, respectively.
    end_to_end_test(Some(3), Some(4))
}

/// This test verifies that aggregations fail as expected if some subset of the
/// batches being aggregated are invalid due to either an invalid signature or
/// an invalid packet file digest.
#[test]
fn aggregation_including_invalid_batch() {
    log_init();

    let pha_tempdir = TempDir::new().unwrap();
    let facilitator_tempdir = TempDir::new().unwrap();

    let instance_name = "fake-instance";
    let aggregation_name = "fake-aggregation-1";
    let date = NaiveDateTime::from_timestamp(2234567890, 654321);
    let start_date = NaiveDateTime::from_timestamp(1234567890, 654321);
    let end_date = NaiveDateTime::from_timestamp(3234567890, 654321);

    let mut ingestor_pub_keys = HashMap::new();
    ingestor_pub_keys.insert(
        default_ingestor_private_key().identifier,
        default_ingestor_public_key(),
    );

    let mut pha_output = SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                pha_tempdir.path().join("ingestion"),
            )),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_key: PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
        drop_nth_packet: None,
    };

    let mut facilitator_output = SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().join("ingestion"),
            )),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_key: PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY)
            .unwrap(),
        drop_nth_packet: None,
    };

    // We generate four batches:
    //   - batches 1 and 2 will have valid ingestion, own and peer batches and
    //     should be summed normally
    //   - batch 3: PHA will sign the peer validation batch sent to facilitator
    //     with the wrong key
    //   - batch 4: facilitator will insert the wrong packet file digest into
    //     the header of the peer validation batch sent to PHA
    let batch_uuids_and_dates = vec![
        (Uuid::new_v4(), date),
        (Uuid::new_v4(), date),
        (Uuid::new_v4(), date),
        (Uuid::new_v4(), date),
    ];

    let mut reference_sums = vec![];

    for (batch_uuid, _) in &batch_uuids_and_dates {
        let reference_sum = generate_ingestion_sample(
            batch_uuid,
            aggregation_name,
            &date,
            10,
            100,
            0.11,
            100,
            100,
            &mut pha_output,
            &mut facilitator_output,
        )
        .unwrap();

        reference_sums.push(reference_sum);
    }

    let mut ingestor_pub_keys = HashMap::new();
    ingestor_pub_keys.insert(
        default_ingestor_private_key().identifier,
        default_ingestor_public_key(),
    );

    // PHA reads ingestion batches from this transport
    let mut pha_ingest_transport = VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: Box::new(LocalFileTransport::new(
                pha_tempdir.path().join("ingestion"),
            )),
            batch_signing_public_keys: ingestor_pub_keys.clone(),
        },
        packet_decryption_keys: vec![
            PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
        ],
    };

    // Facilitator reads ingestion batches from this transport
    let mut facilitator_ingest_transport = VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().join("ingestion"),
            )),
            batch_signing_public_keys: ingestor_pub_keys,
        },
        packet_decryption_keys: vec![
            PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
        ],
    };

    // PHA uses this transport to send correctly signed validation batches to
    // facilitator
    let mut pha_to_facilitator_validation_transport_valid_key = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().join("peer-validation"),
        )),
        batch_signing_key: default_pha_signing_private_key(),
    };

    // PHA uses this transport to send incorrectly signed validation batches to
    // facilitator
    let mut pha_to_facilitator_validation_transport_wrong_key = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().join("peer-validation"),
        )),
        // Intentionally the wrong key
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    // Facilitator uses this transport to send correctly signed validation
    // batches to PHA
    let mut facilitator_to_pha_validation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            pha_tempdir.path().join("peer-validation"),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    // PHA uses this transport to send correctly signed validation batches to
    // itself
    let mut pha_own_validation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            pha_tempdir.path().join("own-validation"),
        )),
        batch_signing_key: default_pha_signing_private_key(),
    };

    // Facilitator uses this transport to send correctly signed validation
    // batches to itself
    let mut facilitator_own_validation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().join("own-validation"),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    // Perform the intake over the batches, on the PHA and then facilitator,
    // tampering with signatures or batch headers as needed.
    for (index, (uuid, _)) in batch_uuids_and_dates.iter().enumerate() {
        let pha_peer_validation_transport = if index == 2 {
            log::info!("pha using wrong key for peer validations");
            &mut pha_to_facilitator_validation_transport_wrong_key
        } else {
            &mut pha_to_facilitator_validation_transport_valid_key
        };

        let mut pha_batch_intaker = BatchIntaker::new(
            aggregation_name,
            &uuid,
            &date,
            &mut pha_ingest_transport,
            &mut pha_own_validation_transport,
            pha_peer_validation_transport,
            true, // is_first
        )
        .unwrap();

        let mut facilitator_batch_intaker = BatchIntaker::new(
            aggregation_name,
            &uuid,
            &date,
            &mut facilitator_ingest_transport,
            &mut facilitator_own_validation_transport,
            &mut facilitator_to_pha_validation_transport,
            false, // is_first
        )
        .unwrap();

        if index == 3 {
            pha_batch_intaker.set_use_bogus_packet_file_digest(true);
            facilitator_batch_intaker.set_use_bogus_packet_file_digest(true);
        }

        pha_batch_intaker.generate_validation_share(|| {}).unwrap();
        facilitator_batch_intaker
            .generate_validation_share(|| {})
            .unwrap();
    }

    // Batch signing keys advertised by PHA
    let mut pha_pub_keys = HashMap::new();
    pha_pub_keys.insert(
        default_pha_signing_private_key().identifier,
        default_pha_signing_public_key(),
    );

    // Batch signing keys advertised by facilitator
    let mut facilitator_pub_keys = HashMap::new();
    facilitator_pub_keys.insert(
        default_facilitator_signing_private_key().identifier,
        default_facilitator_signing_public_key(),
    );

    // PHA uses this transport to read its own validations
    let mut pha_own_validation_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            pha_tempdir.path().join("own-validation"),
        )),
        batch_signing_public_keys: pha_pub_keys.clone(),
    };

    // PHA uses this transport to read facilitator's validations
    let mut pha_peer_validation_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            pha_tempdir.path().join("peer-validation"),
        )),
        batch_signing_public_keys: facilitator_pub_keys.clone(),
    };

    // Facilitator uses this transport to read its own validations
    let mut facilitator_own_validation_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().join("own-validation"),
        )),
        batch_signing_public_keys: facilitator_pub_keys.clone(),
    };

    // Facilitator uses this transport to read PHA's validations
    let mut facilitator_peer_validation_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().join("peer-validation"),
        )),
        batch_signing_public_keys: pha_pub_keys.clone(),
    };

    // PHA uses this transport to send sum parts
    let mut pha_aggregation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
        batch_signing_key: default_pha_signing_private_key(),
    };

    // Facilitator uses this transport to send sum parts
    let mut facilitator_aggregation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().to_path_buf(),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    // Perform the aggregation on PHA and facilitator
    let err = BatchAggregator::new(
        instance_name,
        aggregation_name,
        &start_date,
        &end_date,
        true, // is_first
        &mut pha_ingest_transport,
        &mut pha_own_validation_transport,
        &mut pha_peer_validation_transport,
        &mut pha_aggregation_transport,
    )
    .unwrap()
    .generate_sum_part(&batch_uuids_and_dates, || {})
    .unwrap_err();

    assert!(matches!(err, Error::BadPeerValidationBatch(_)));

    let err = BatchAggregator::new(
        instance_name,
        aggregation_name,
        &start_date,
        &end_date,
        false, // is_first
        &mut facilitator_ingest_transport,
        &mut facilitator_own_validation_transport,
        &mut facilitator_peer_validation_transport,
        &mut facilitator_aggregation_transport,
    )
    .unwrap()
    .generate_sum_part(&batch_uuids_and_dates, || {})
    .unwrap_err();

    assert!(matches!(err, Error::BadPeerValidationBatch(_)));
}

fn end_to_end_test(drop_nth_pha: Option<usize>, drop_nth_facilitator: Option<usize>) {
    log_init();
    let pha_tempdir = TempDir::new().unwrap();
    let pha_copy_tempdir = TempDir::new().unwrap();
    let facilitator_tempdir = TempDir::new().unwrap();
    let facilitator_copy_tempdir = TempDir::new().unwrap();

    let instance_name = "fake-instance";
    let aggregation_name = "fake-aggregation-1".to_owned();
    let date = NaiveDateTime::from_timestamp(2234567890, 654321);
    let start_date = NaiveDateTime::from_timestamp(1234567890, 654321);
    let end_date = NaiveDateTime::from_timestamp(3234567890, 654321);

    let batch_1_uuid = Uuid::new_v4();
    let batch_2_uuid = Uuid::new_v4();

    let mut ingestor_pub_keys = HashMap::new();
    ingestor_pub_keys.insert(
        default_ingestor_private_key().identifier,
        default_ingestor_public_key(),
    );

    let mut pha_output = SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_key: PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
        drop_nth_packet: drop_nth_pha,
    };

    let mut facilitator_output = SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().to_path_buf(),
            )),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_key: PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY)
            .unwrap(),
        drop_nth_packet: drop_nth_facilitator,
    };

    let first_batch_packet_count = 16;

    let batch_1_reference_sum = generate_ingestion_sample(
        &batch_1_uuid,
        &aggregation_name,
        &date,
        10,
        first_batch_packet_count,
        0.11,
        100,
        100,
        &mut pha_output,
        &mut facilitator_output,
    )
    .unwrap();

    let batch_2_reference_sum = generate_ingestion_sample(
        &batch_2_uuid,
        &aggregation_name,
        &date,
        10,
        14,
        0.11,
        100,
        100,
        &mut pha_output,
        &mut facilitator_output,
    )
    .unwrap();

    let mut ingestor_pub_keys = HashMap::new();
    ingestor_pub_keys.insert(
        default_ingestor_private_key().identifier,
        default_ingestor_public_key(),
    );
    let mut pha_ingest_transport = VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
            batch_signing_public_keys: ingestor_pub_keys.clone(),
        },
        packet_decryption_keys: vec![
            PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
        ],
    };

    let mut facilitator_ingest_transport = VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().to_path_buf(),
            )),
            batch_signing_public_keys: ingestor_pub_keys,
        },
        packet_decryption_keys: vec![
            PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
        ],
    };

    let mut pha_peer_validate_signable_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
        batch_signing_key: default_pha_signing_private_key(),
    };

    let mut facilitator_peer_validate_signable_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().to_path_buf(),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    let mut pha_own_validate_signable_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            pha_copy_tempdir.path().to_path_buf(),
        )),
        batch_signing_key: default_pha_signing_private_key(),
    };

    let mut facilitator_own_validate_signable_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_copy_tempdir.path().to_path_buf(),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    let mut intake_callback_count = 0;
    let mut batch_intaker = BatchIntaker::new(
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_peer_validate_signable_transport,
        &mut pha_own_validate_signable_transport,
        true,
    )
    .unwrap();
    batch_intaker.set_callback_cadence(2);
    batch_intaker
        .generate_validation_share(|| intake_callback_count += 1)
        .unwrap();

    assert_eq!(
        intake_callback_count,
        (first_batch_packet_count - batch_1_reference_sum.pha_dropped_packets.len()) / 2
    );

    BatchIntaker::new(
        &aggregation_name,
        &batch_2_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_peer_validate_signable_transport,
        &mut pha_own_validate_signable_transport,
        true,
    )
    .unwrap()
    .generate_validation_share(|| {})
    .unwrap();

    BatchIntaker::new(
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut facilitator_ingest_transport,
        &mut facilitator_peer_validate_signable_transport,
        &mut facilitator_own_validate_signable_transport,
        false,
    )
    .unwrap()
    .generate_validation_share(|| {})
    .unwrap();

    BatchIntaker::new(
        &aggregation_name,
        &batch_2_uuid,
        &date,
        &mut facilitator_ingest_transport,
        &mut facilitator_peer_validate_signable_transport,
        &mut facilitator_own_validate_signable_transport,
        false,
    )
    .unwrap()
    .generate_validation_share(|| {})
    .unwrap();

    let batch_ids_and_dates = vec![(batch_1_uuid, date), (batch_2_uuid, date)];

    let mut pha_pub_keys = HashMap::new();
    pha_pub_keys.insert(
        default_pha_signing_private_key().identifier,
        default_pha_signing_public_key(),
    );

    let mut pha_validate_verifiable_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
        batch_signing_public_keys: pha_pub_keys.clone(),
    };

    let mut facilitator_pub_keys = HashMap::new();
    facilitator_pub_keys.insert(
        default_facilitator_signing_private_key().identifier,
        default_facilitator_signing_public_key(),
    );
    let mut facilitator_validate_verifiable_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().to_path_buf(),
        )),
        batch_signing_public_keys: facilitator_pub_keys.clone(),
    };

    let mut pha_aggregation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
        batch_signing_key: default_pha_signing_private_key(),
    };

    let mut aggregation_callback_count = 0;
    BatchAggregator::new(
        instance_name,
        &aggregation_name,
        &start_date,
        &end_date,
        true,
        &mut pha_ingest_transport,
        &mut pha_validate_verifiable_transport,
        &mut facilitator_validate_verifiable_transport,
        &mut pha_aggregation_transport,
    )
    .unwrap()
    .generate_sum_part(&batch_ids_and_dates, || aggregation_callback_count += 1)
    .unwrap();

    assert_eq!(aggregation_callback_count, 2);

    let mut facilitator_aggregation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().to_path_buf(),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };

    let mut aggregation_callback_count = 0;
    BatchAggregator::new(
        instance_name,
        &aggregation_name,
        &start_date,
        &end_date,
        false,
        &mut facilitator_ingest_transport,
        &mut facilitator_validate_verifiable_transport,
        &mut pha_validate_verifiable_transport,
        &mut facilitator_aggregation_transport,
    )
    .unwrap()
    .generate_sum_part(&batch_ids_and_dates, || aggregation_callback_count += 1)
    .unwrap();

    assert_eq!(aggregation_callback_count, 2);

    let mut pha_aggregation_batch_reader: BatchReader<'_, SumPart, InvalidPacket> =
        BatchReader::new(
            Batch::new_sum(
                instance_name,
                &aggregation_name,
                &start_date,
                &end_date,
                true,
            ),
            &mut *pha_aggregation_transport.transport,
        );
    let pha_sum_part = pha_aggregation_batch_reader.header(&pha_pub_keys).unwrap();
    assert_eq!(
        pha_sum_part.total_individual_clients,
        batch_1_reference_sum.contributions as i64 + batch_2_reference_sum.contributions as i64
    );
    let pha_sum_fields = pha_sum_part.sum().unwrap();

    let mut facilitator_aggregation_batch_reader: BatchReader<'_, SumPart, InvalidPacket> =
        BatchReader::new(
            Batch::new_sum(
                instance_name,
                &aggregation_name,
                &start_date,
                &end_date,
                false,
            ),
            &mut *facilitator_aggregation_transport.transport,
        );
    let facilitator_sum_part = facilitator_aggregation_batch_reader
        .header(&facilitator_pub_keys)
        .unwrap();
    assert_eq!(
        facilitator_sum_part.total_individual_clients,
        batch_1_reference_sum.contributions as i64 + batch_2_reference_sum.contributions as i64
    );
    let facilitator_sum_fields = facilitator_sum_part.sum().unwrap();

    let reconstructed = reconstruct_shares(&facilitator_sum_fields, &pha_sum_fields).unwrap();

    let reference_sum =
        reconstruct_shares(&batch_1_reference_sum.sum, &batch_2_reference_sum.sum).unwrap();
    assert_eq!(
        reconstructed, reference_sum,
        "reconstructed shares do not match original data.\npha sum: {:?}\n
            facilitator sum: {:?}\nreconstructed sum: {:?}\nreference sum: {:?}",
        pha_sum_fields, facilitator_sum_fields, reconstructed, reference_sum
    );

    assert_eq!(
        facilitator_sum_part.total_individual_clients, pha_sum_part.total_individual_clients,
        "facilitator sum part total individual clients does not match the pha sum part total individual clients\n\
        \tfacilitator clients: {}\n\tpha clients: {}",
        facilitator_sum_part.total_individual_clients, pha_sum_part.total_individual_clients
    );

    check_invalid_packets(
        &batch_1_reference_sum.facilitator_dropped_packets,
        &batch_2_reference_sum.facilitator_dropped_packets,
        &mut pha_aggregation_batch_reader,
        &pha_sum_part,
    );

    check_invalid_packets(
        &batch_1_reference_sum.pha_dropped_packets,
        &batch_2_reference_sum.pha_dropped_packets,
        &mut facilitator_aggregation_batch_reader,
        &facilitator_sum_part,
    );
}

fn check_invalid_packets(
    peer_dropped_packets_1: &[Uuid],
    peer_dropped_packets_2: &[Uuid],
    batch_reader: &mut BatchReader<'_, SumPart, InvalidPacket>,
    sum_part_header: &SumPart,
) {
    if !peer_dropped_packets_1.is_empty() || !peer_dropped_packets_2.is_empty() {
        // Check the packets that were marked invalid by either data share
        // processor against the ones dropped from the other's ingestion batches
        let mut dropped_packets = HashSet::new();
        for dropped in peer_dropped_packets_1 {
            dropped_packets.insert(dropped);
        }
        for dropped in peer_dropped_packets_2 {
            dropped_packets.insert(dropped);
        }
        let mut invalid_packet_reader = batch_reader.packet_file_reader(sum_part_header).unwrap();
        loop {
            match InvalidPacket::read(&mut invalid_packet_reader) {
                Ok(packet) => assert!(dropped_packets.contains(&packet.uuid)),
                Err(Error::Eof) => break,
                Err(err) => assert!(false, "error reading invalid packet {}", err),
            }
        }
    } else {
        assert!(batch_reader.packet_file_reader(sum_part_header).is_err());
    }
}
