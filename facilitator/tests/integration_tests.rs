use chrono::NaiveDateTime;
use facilitator::{
    aggregation::BatchAggregator,
    batch::{Batch, BatchReader},
    idl::{IngestionDataSharePacket, SumPart},
    intake::BatchIntaker,
    sample::generate_ingestion_sample,
    test_utils::{
        default_facilitator_signing_private_key, default_facilitator_signing_public_key,
        default_ingestor_private_key, default_ingestor_public_key, default_pha_signing_private_key,
        default_pha_signing_public_key, DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY,
        DEFAULT_PHA_ECIES_PRIVATE_KEY,
    },
    transport::{
        LocalFileTransport, SignableTransport, VerifiableAndDecryptableTransport,
        VerifiableTransport,
    },
};
use prio::{encrypt::PrivateKey, util::reconstruct_shares};
use std::collections::HashMap;
use uuid::Uuid;

#[test]
fn end_to_end() {
    let pha_tempdir = tempfile::TempDir::new().unwrap();
    let pha_copy_tempdir = tempfile::TempDir::new().unwrap();
    let facilitator_tempdir = tempfile::TempDir::new().unwrap();
    let facilitator_copy_tempdir = tempfile::TempDir::new().unwrap();

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

    let batch_1_reference_sum = generate_ingestion_sample(
        &mut LocalFileTransport::new(pha_tempdir.path().to_path_buf()),
        &mut LocalFileTransport::new(facilitator_tempdir.path().to_path_buf()),
        &batch_1_uuid,
        &aggregation_name,
        &date,
        &PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
        &PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
        &default_ingestor_private_key(),
        10,
        10,
        0.11,
        100,
        100,
    )
    .unwrap();

    let batch_2_reference_sum = generate_ingestion_sample(
        &mut LocalFileTransport::new(pha_tempdir.path().to_path_buf()),
        &mut LocalFileTransport::new(facilitator_tempdir.path().to_path_buf()),
        &batch_2_uuid,
        &aggregation_name,
        &date,
        &PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
        &PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
        &default_ingestor_private_key(),
        10,
        10,
        0.11,
        100,
        100,
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
            batch_signing_public_keys: ingestor_pub_keys.clone(),
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

    BatchIntaker::new(
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_peer_validate_signable_transport,
        &mut pha_own_validate_signable_transport,
        true,
    )
    .unwrap()
    .generate_validation_share()
    .unwrap();

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
    .generate_validation_share()
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
    .generate_validation_share()
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
    .generate_validation_share()
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
    BatchAggregator::new(
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
    .generate_sum_part(&batch_ids_and_dates)
    .unwrap();

    let mut facilitator_aggregation_transport = SignableTransport {
        transport: Box::new(LocalFileTransport::new(
            facilitator_tempdir.path().to_path_buf(),
        )),
        batch_signing_key: default_facilitator_signing_private_key(),
    };
    BatchAggregator::new(
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
    .generate_sum_part(&batch_ids_and_dates)
    .unwrap();

    let mut pha_aggregation_batch_reader: BatchReader<'_, SumPart, IngestionDataSharePacket> =
        BatchReader::new(
            Batch::new_sum(&aggregation_name, &start_date, &end_date, true),
            &mut *pha_aggregation_transport.transport,
        );
    let pha_sum_part = pha_aggregation_batch_reader.header(&pha_pub_keys).unwrap();
    let pha_sum_fields = pha_sum_part.sum().unwrap();

    let pha_invalid_packet_reader = pha_aggregation_batch_reader.packet_file_reader(&pha_sum_part);
    assert!(
        pha_invalid_packet_reader.is_err(),
        "should get no invalid packet reader when all packets were OK"
    );

    let mut facilitator_aggregation_batch_reader: BatchReader<
        '_,
        SumPart,
        IngestionDataSharePacket,
    > = BatchReader::new(
        Batch::new_sum(&aggregation_name, &start_date, &end_date, false),
        &mut *facilitator_aggregation_transport.transport,
    );
    let facilitator_sum_part = facilitator_aggregation_batch_reader
        .header(&facilitator_pub_keys)
        .unwrap();
    let facilitator_sum_fields = facilitator_sum_part.sum().unwrap();

    let facilitator_invalid_packet_reader =
        facilitator_aggregation_batch_reader.packet_file_reader(&facilitator_sum_part);
    assert!(
        facilitator_invalid_packet_reader.is_err(),
        "should get no invalid packet reader when all packets were OK"
    );

    let reconstructed = reconstruct_shares(&facilitator_sum_fields, &pha_sum_fields).unwrap();

    let reference_sum = reconstruct_shares(&batch_1_reference_sum, &batch_2_reference_sum).unwrap();
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

    assert_eq!(
        reconstructed.len() as i64,
        facilitator_sum_part.total_individual_clients,
        "Total individual clients does not match the length of sum\n\
        \ttotal individual clients: {}\n\tlength of sum: {}",
        facilitator_sum_part.total_individual_clients,
        reconstructed.len()
    );
}
