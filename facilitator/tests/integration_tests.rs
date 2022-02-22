use chrono::NaiveDateTime;
use facilitator::{
    aggregation::BatchAggregator,
    batch::{Batch, BatchReader},
    idl::InvalidPacket,
    intake::BatchIntaker,
    logging::setup_test_logging,
    sample::{SampleGenerator, SampleOutput},
    test_utils::{
        bogus_packet_decryption_private_key, bogus_packet_encryption_public_key,
        default_facilitator_packet_decryption_private_key,
        default_facilitator_packet_encryption_public_key, default_facilitator_signing_private_key,
        default_facilitator_signing_public_key, default_ingestor_private_key,
        default_ingestor_public_key, default_pha_packet_decryption_private_key,
        default_pha_packet_encryption_public_key, default_pha_signing_private_key,
        default_pha_signing_public_key,
    },
    transport::{
        LocalFileTransport, SignableTransport, VerifiableAndDecryptableTransport,
        VerifiableTransport,
    },
};
use prio::{encrypt::PublicKey, util::reconstruct_shares};
use slog::info;
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;
use uuid::Uuid;

const TRACE_ID: Uuid = Uuid::from_bytes([97; 16]);

#[test]
fn end_to_end() {
    end_to_end_test(EndToEndTestOptions {
        drop_nth_pha_packet: None,
        drop_nth_facilitator_packet: None,
        use_wrong_encryption_key_for_second_batch: false,
    })
}

#[test]
fn inconsistent_ingestion_batches() {
    // Have sample generation drop every third and every fourth packet from the
    // PHA and facilitator ingestion batches, respectively.
    end_to_end_test(EndToEndTestOptions {
        drop_nth_pha_packet: Some(3),
        drop_nth_facilitator_packet: Some(4),
        use_wrong_encryption_key_for_second_batch: false,
    })
}

#[test]
fn wrong_encryption_key() {
    // Use the wrong encryption key for the second batch. The first batch should
    // still get aggregated.
    end_to_end_test(EndToEndTestOptions {
        drop_nth_pha_packet: None,
        drop_nth_facilitator_packet: None,
        use_wrong_encryption_key_for_second_batch: true,
    })
}

/// This test verifies that aggregations fail as expected if some subset of the
/// batches being aggregated are invalid due to either an invalid signature or
/// an invalid packet file digest.
#[test]
fn aggregation_including_invalid_batch() {
    let logger = setup_test_logging();

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

    let pha_output = SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                pha_tempdir.path().join("ingestion"),
            )),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_public_key: default_pha_packet_encryption_public_key(),
        drop_nth_packet: None,
    };

    let facilitator_output = SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().join("ingestion"),
            )),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_public_key: default_facilitator_packet_encryption_public_key(),
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

    let sample_generator = SampleGenerator::new(
        aggregation_name,
        10,
        0.11,
        100,
        100,
        &pha_output,
        &facilitator_output,
        &logger,
    );

    for (batch_uuid, date) in &batch_uuids_and_dates {
        let reference_sum = sample_generator
            .generate_ingestion_sample(&TRACE_ID, batch_uuid, date, 100)
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
            default_pha_packet_decryption_private_key(),
            default_facilitator_packet_decryption_private_key(),
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
            default_pha_packet_decryption_private_key(),
            default_facilitator_packet_decryption_private_key(),
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

    // Perform the intake over the batches, on the PHA and then facilitator,
    // tampering with signatures or batch headers as needed.
    for (index, (uuid, _)) in batch_uuids_and_dates.iter().enumerate() {
        let pha_peer_validation_transport = if index == 2 {
            info!(logger, "pha using wrong key for peer validations");
            &mut pha_to_facilitator_validation_transport_wrong_key
        } else {
            &mut pha_to_facilitator_validation_transport_valid_key
        };

        let mut pha_batch_intaker = BatchIntaker::new(
            &TRACE_ID,
            aggregation_name,
            uuid,
            &date,
            &mut pha_ingest_transport,
            pha_peer_validation_transport,
            true,  // is_first
            false, // permissive
            &logger,
        )
        .unwrap();

        let mut facilitator_batch_intaker = BatchIntaker::new(
            &TRACE_ID,
            aggregation_name,
            uuid,
            &date,
            &mut facilitator_ingest_transport,
            &mut facilitator_to_pha_validation_transport,
            false, // is_first
            false, // permissive
            &logger,
        )
        .unwrap();

        if index == 3 {
            pha_batch_intaker.set_use_bogus_packet_file_digest(true);
            facilitator_batch_intaker.set_use_bogus_packet_file_digest(true);
        }

        pha_batch_intaker.generate_validation_share(|_| {}).unwrap();
        facilitator_batch_intaker
            .generate_validation_share(|_| {})
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

    // PHA uses this transport to read facilitator's validations
    let mut pha_peer_validation_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            pha_tempdir.path().join("peer-validation"),
        )),
        batch_signing_public_keys: facilitator_pub_keys.clone(),
    };

    // Facilitator uses this transport to read PHA's validations
    let mut facilitator_peer_validation_transport = VerifiableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().join("peer-validation"),
        )),
        batch_signing_public_keys: pha_pub_keys,
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
        &TRACE_ID,
        instance_name,
        aggregation_name,
        &start_date,
        &end_date,
        true,  // is_first
        false, // permissive
        &mut pha_ingest_transport,
        &mut pha_peer_validation_transport,
        &mut pha_aggregation_transport,
        &logger,
    )
    .unwrap()
    .generate_sum_part(&batch_uuids_and_dates, |_| {})
    .unwrap_err();
    // Ideally we would be able to match on a variant in an error enum to check
    // what the failure was but for now check the error description
    assert!(err.to_string().contains("packet file digest in header"));

    let err = BatchAggregator::new(
        &TRACE_ID,
        instance_name,
        aggregation_name,
        &start_date,
        &end_date,
        false, // is_first
        false, // permissive
        &mut facilitator_ingest_transport,
        &mut facilitator_peer_validation_transport,
        &mut facilitator_aggregation_transport,
        &logger,
    )
    .unwrap()
    .generate_sum_part(&batch_uuids_and_dates, |_| {})
    .unwrap_err();
    assert!(err
        .to_string()
        .contains("key identifier default-facilitator-signing-key not present in key map"));
}

fn sample_output(
    temp_dir: &TempDir,
    packet_encryption_public_key: PublicKey,
    drop_nth_packet: Option<usize>,
) -> SampleOutput {
    SampleOutput {
        transport: SignableTransport {
            transport: Box::new(LocalFileTransport::new(temp_dir.path().to_path_buf())),
            batch_signing_key: default_ingestor_private_key(),
        },
        packet_encryption_public_key,
        drop_nth_packet,
    }
}

struct EndToEndTestOptions {
    drop_nth_pha_packet: Option<usize>,
    drop_nth_facilitator_packet: Option<usize>,
    use_wrong_encryption_key_for_second_batch: bool,
}

fn end_to_end_test(test_options: EndToEndTestOptions) {
    let logger = setup_test_logging();
    let pha_tempdir = TempDir::new().unwrap();
    let facilitator_tempdir = TempDir::new().unwrap();

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

    let batch_1_pha_output = sample_output(
        &pha_tempdir,
        default_pha_packet_encryption_public_key(),
        test_options.drop_nth_pha_packet,
    );
    let batch_2_pha_output = sample_output(
        &pha_tempdir,
        if test_options.use_wrong_encryption_key_for_second_batch {
            bogus_packet_encryption_public_key()
        } else {
            default_pha_packet_encryption_public_key()
        },
        test_options.drop_nth_pha_packet,
    );

    let batch_1_facilitator_output = sample_output(
        &facilitator_tempdir,
        default_facilitator_packet_encryption_public_key(),
        test_options.drop_nth_facilitator_packet,
    );
    let batch_2_facilitator_output = sample_output(
        &facilitator_tempdir,
        if test_options.use_wrong_encryption_key_for_second_batch {
            bogus_packet_encryption_public_key()
        } else {
            default_facilitator_packet_encryption_public_key()
        },
        test_options.drop_nth_facilitator_packet,
    );

    let first_batch_packet_count = 16;

    let batch_1_sample_generator = SampleGenerator::new(
        &aggregation_name,
        10,
        0.11,
        100,
        100,
        &batch_1_pha_output,
        &batch_1_facilitator_output,
        &logger,
    );

    let batch_1_reference_sum = batch_1_sample_generator
        .generate_ingestion_sample(&TRACE_ID, &batch_1_uuid, &date, first_batch_packet_count)
        .unwrap();

    let batch_2_sample_generator = SampleGenerator::new(
        &aggregation_name,
        10,
        0.11,
        100,
        100,
        &batch_2_pha_output,
        &batch_2_facilitator_output,
        &logger,
    );

    let batch_2_reference_sum = batch_2_sample_generator
        .generate_ingestion_sample(&TRACE_ID, &batch_2_uuid, &date, 14)
        .unwrap();

    let mut ingestor_pub_keys = HashMap::new();
    ingestor_pub_keys.insert(
        default_ingestor_private_key().identifier,
        default_ingestor_public_key(),
    );
    let packet_decryption_keys = vec![
        default_pha_packet_decryption_private_key(),
        default_facilitator_packet_decryption_private_key(),
        // We provide the bogus decryption key at intake time because we want
        // to test decryption failures during aggregation
        bogus_packet_decryption_private_key(),
    ];

    let mut pha_ingest_transport = VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: Box::new(LocalFileTransport::new(pha_tempdir.path().to_path_buf())),
            batch_signing_public_keys: ingestor_pub_keys.clone(),
        },
        packet_decryption_keys: packet_decryption_keys.clone(),
    };

    let mut facilitator_ingest_transport = VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: Box::new(LocalFileTransport::new(
                facilitator_tempdir.path().to_path_buf(),
            )),
            batch_signing_public_keys: ingestor_pub_keys,
        },
        packet_decryption_keys,
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

    let mut intake_callback_count = 0;
    let mut batch_intaker = BatchIntaker::new(
        &TRACE_ID,
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_peer_validate_signable_transport,
        true,
        false,
        &logger,
    )
    .unwrap();
    batch_intaker.set_callback_cadence(2);
    batch_intaker
        .generate_validation_share(|_| intake_callback_count += 1)
        .unwrap();

    assert_eq!(
        intake_callback_count,
        (first_batch_packet_count - batch_1_reference_sum.pha_dropped_packets.len()) / 2
    );

    BatchIntaker::new(
        &TRACE_ID,
        &aggregation_name,
        &batch_2_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_peer_validate_signable_transport,
        true,
        false,
        &logger,
    )
    .unwrap()
    .generate_validation_share(|_| {})
    .unwrap();

    BatchIntaker::new(
        &TRACE_ID,
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut facilitator_ingest_transport,
        &mut facilitator_peer_validate_signable_transport,
        false,
        false,
        &logger,
    )
    .unwrap()
    .generate_validation_share(|_| {})
    .unwrap();

    BatchIntaker::new(
        &TRACE_ID,
        &aggregation_name,
        &batch_2_uuid,
        &date,
        &mut facilitator_ingest_transport,
        &mut facilitator_peer_validate_signable_transport,
        false,
        false,
        &logger,
    )
    .unwrap()
    .generate_validation_share(|_| {})
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

    let packet_decryption_keys = vec![
        default_pha_packet_decryption_private_key(),
        default_facilitator_packet_decryption_private_key(),
        // No more bogus decryption key
    ];

    pha_ingest_transport.packet_decryption_keys = packet_decryption_keys.clone();
    facilitator_ingest_transport.packet_decryption_keys = packet_decryption_keys;

    let mut aggregation_callback_count = 0;
    BatchAggregator::new(
        &TRACE_ID,
        instance_name,
        &aggregation_name,
        &start_date,
        &end_date,
        true,
        false,
        &mut pha_ingest_transport,
        &mut facilitator_validate_verifiable_transport,
        &mut pha_aggregation_transport,
        &logger,
    )
    .unwrap()
    .generate_sum_part(&batch_ids_and_dates, |_| aggregation_callback_count += 1)
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
        &TRACE_ID,
        instance_name,
        &aggregation_name,
        &start_date,
        &end_date,
        false,
        false,
        &mut facilitator_ingest_transport,
        &mut pha_validate_verifiable_transport,
        &mut facilitator_aggregation_transport,
        &logger,
    )
    .unwrap()
    .generate_sum_part(&batch_ids_and_dates, |_| aggregation_callback_count += 1)
    .unwrap();

    assert_eq!(aggregation_callback_count, 2);

    let pha_aggregation_batch_reader = BatchReader::new(
        Batch::new_sum(
            instance_name,
            &aggregation_name,
            &start_date,
            &end_date,
            true,
        ),
        &*pha_aggregation_transport.transport,
        false,
        &TRACE_ID,
        &logger,
    );
    let (pha_sum_part, pha_invalid_packets) =
        pha_aggregation_batch_reader.read(&pha_pub_keys).unwrap();
    let pha_sum_fields = pha_sum_part.sum().unwrap();

    let facilitator_aggregation_batch_reader = BatchReader::new(
        Batch::new_sum(
            instance_name,
            &aggregation_name,
            &start_date,
            &end_date,
            false,
        ),
        &*facilitator_aggregation_transport.transport,
        false, // permissive
        &TRACE_ID,
        &logger,
    );

    let (facilitator_sum_part, facilitator_invalid_packets) = facilitator_aggregation_batch_reader
        .read(&facilitator_pub_keys)
        .unwrap();
    let facilitator_sum_fields = facilitator_sum_part.sum().unwrap();

    let reconstructed = reconstruct_shares(&facilitator_sum_fields, &pha_sum_fields).unwrap();

    // If the second batch was encrypted under the wrong, key, then the
    // aggregate should only include the sum over the first batch, the first
    // batch's UUID, and the count of contributions from the first batch
    let (reference_sum, expected_batch_uuids, expected_total_individual_clients) = if test_options
        .use_wrong_encryption_key_for_second_batch
    {
        (
            batch_1_reference_sum.sum,
            vec![batch_1_uuid],
            batch_1_reference_sum.contributions as i64,
        )
    } else {
        (
            reconstruct_shares(&batch_1_reference_sum.sum, &batch_2_reference_sum.sum).unwrap(),
            vec![batch_1_uuid, batch_2_uuid],
            batch_1_reference_sum.contributions as i64 + batch_2_reference_sum.contributions as i64,
        )
    };
    assert_eq!(
        reconstructed, reference_sum,
        "reconstructed shares do not match original data.\npha sum: {:?}\n
            facilitator sum: {:?}\nreconstructed sum: {:?}\nreference sum: {:?}",
        pha_sum_fields, facilitator_sum_fields, reconstructed, reference_sum
    );

    assert_eq!(expected_batch_uuids, facilitator_sum_part.batch_uuids);
    assert_eq!(expected_batch_uuids, pha_sum_part.batch_uuids);

    assert_eq!(
        pha_sum_part.total_individual_clients,
        expected_total_individual_clients
    );

    assert_eq!(
        facilitator_sum_part.total_individual_clients,
        expected_total_individual_clients
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
        pha_invalid_packets,
    );

    check_invalid_packets(
        &batch_1_reference_sum.pha_dropped_packets,
        &batch_2_reference_sum.pha_dropped_packets,
        facilitator_invalid_packets,
    );
}

fn check_invalid_packets(
    peer_dropped_packets_1: &[Uuid],
    peer_dropped_packets_2: &[Uuid],
    invalid_packets: Vec<InvalidPacket>,
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
        for packet in invalid_packets {
            assert!(dropped_packets.contains(&packet.uuid));
        }
    } else {
        assert!(invalid_packets.is_empty());
    }
}
