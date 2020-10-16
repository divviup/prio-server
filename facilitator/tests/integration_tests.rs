use chrono::NaiveDateTime;
use facilitator::{
    aggregation::BatchAggregator,
    batch::{Batch, BatchReader},
    idl::{IngestionDataSharePacket, SumPart},
    intake::BatchIntaker,
    sample::generate_ingestion_sample,
    test_utils::{
        default_facilitator_signing_private_key, default_ingestor_private_key,
        default_ingestor_private_key_raw, default_pha_signing_private_key,
        DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY, DEFAULT_PHA_ECIES_PRIVATE_KEY,
    },
    transport::LocalFileTransport,
};
use prio::{encrypt::PrivateKey, util::reconstruct_shares};
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED,
    ECDSA_P256_SHA256_FIXED_SIGNING,
};
use uuid::Uuid;

#[test]
fn end_to_end() {
    let pha_tempdir = tempfile::TempDir::new().unwrap();
    let facilitator_tempdir = tempfile::TempDir::new().unwrap();

    let aggregation_name = "fake-aggregation-1".to_owned();
    let date = NaiveDateTime::from_timestamp(2234567890, 654321);
    let start_date = NaiveDateTime::from_timestamp(1234567890, 654321);
    let end_date = NaiveDateTime::from_timestamp(3234567890, 654321);

    let batch_1_uuid = Uuid::new_v4();
    let batch_2_uuid = Uuid::new_v4();

    let mut pha_ingest_transport = LocalFileTransport::new(pha_tempdir.path().to_path_buf());
    let mut facilitator_ingest_transport =
        LocalFileTransport::new(facilitator_tempdir.path().to_path_buf());
    let mut pha_validate_transport = LocalFileTransport::new(pha_tempdir.path().to_path_buf());
    let mut facilitator_validate_transport =
        LocalFileTransport::new(facilitator_tempdir.path().to_path_buf());
    let mut aggregation_transport = LocalFileTransport::new(pha_tempdir.path().to_path_buf());

    let pha_ecies_key = PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap();
    let facilitator_ecies_key =
        PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap();
    let ingestor_pub_key = UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_FIXED,
        default_ingestor_private_key()
            .public_key()
            .as_ref()
            .to_vec(),
    );
    let pha_signing_key = EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &default_pha_signing_private_key(),
    )
    .unwrap();
    let pha_pub_signing_key = UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_FIXED,
        pha_signing_key.public_key().as_ref().to_vec(),
    );
    let facilitator_signing_key = default_facilitator_signing_private_key();
    let facilitator_pub_signing_key = UnparsedPublicKey::new(
        &ECDSA_P256_SHA256_FIXED,
        facilitator_signing_key.public_key().as_ref().to_vec(),
    );

    let batch_1_reference_sum = generate_ingestion_sample(
        &mut pha_ingest_transport,
        &mut facilitator_ingest_transport,
        &batch_1_uuid,
        &aggregation_name,
        &date,
        &pha_ecies_key,
        &facilitator_ecies_key,
        &default_ingestor_private_key_raw(),
        10,
        10,
        0.11,
        100,
        100,
    );
    assert!(
        batch_1_reference_sum.is_ok(),
        "failed to generate first sample: {:?}",
        batch_1_reference_sum.err()
    );

    let batch_2_reference_sum = generate_ingestion_sample(
        &mut pha_ingest_transport,
        &mut facilitator_ingest_transport,
        &batch_2_uuid,
        &aggregation_name,
        &date,
        &pha_ecies_key,
        &facilitator_ecies_key,
        &default_ingestor_private_key_raw(),
        10,
        10,
        0.11,
        100,
        100,
    );
    assert!(
        batch_2_reference_sum.is_ok(),
        "failed to generate second sample: {:?}",
        batch_2_reference_sum.err()
    );

    let res = BatchIntaker::new(
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_validate_transport,
        true,
        &pha_ecies_key,
        &pha_signing_key,
        &ingestor_pub_key,
    )
    .unwrap()
    .generate_validation_share();
    assert!(
        res.is_ok(),
        "PHA failed to generate validation: {:?}",
        res.err()
    );

    let res = BatchIntaker::new(
        &aggregation_name,
        &batch_2_uuid,
        &date,
        &mut pha_ingest_transport,
        &mut pha_validate_transport,
        true,
        &pha_ecies_key,
        &pha_signing_key,
        &ingestor_pub_key,
    )
    .unwrap()
    .generate_validation_share();
    assert!(
        res.is_ok(),
        "PHA failed to generate validation: {:?}",
        res.err()
    );

    let res = BatchIntaker::new(
        &aggregation_name,
        &batch_1_uuid,
        &date,
        &mut facilitator_ingest_transport,
        &mut facilitator_validate_transport,
        false,
        &facilitator_ecies_key,
        &facilitator_signing_key,
        &ingestor_pub_key,
    )
    .unwrap()
    .generate_validation_share();
    assert!(
        res.is_ok(),
        "facilitator failed to generate validation: {:?}",
        res.err()
    );

    let res = BatchIntaker::new(
        &aggregation_name,
        &batch_2_uuid,
        &date,
        &mut facilitator_ingest_transport,
        &mut facilitator_validate_transport,
        false,
        &facilitator_ecies_key,
        &facilitator_signing_key,
        &ingestor_pub_key,
    )
    .unwrap()
    .generate_validation_share();
    assert!(
        res.is_ok(),
        "facilitator failed to generate validation: {:?}",
        res.err()
    );

    let batch_ids_and_dates = vec![(batch_1_uuid, date), (batch_2_uuid, date)];

    let res = BatchAggregator::new(
        &aggregation_name,
        &start_date,
        &end_date,
        true,
        &mut pha_ingest_transport,
        &mut pha_validate_transport,
        &mut facilitator_validate_transport,
        &mut aggregation_transport,
        &ingestor_pub_key,
        &pha_signing_key,
        &facilitator_pub_signing_key,
        &pha_ecies_key,
    )
    .unwrap()
    .generate_sum_part(&batch_ids_and_dates);
    assert!(
        res.is_ok(),
        "PHA failed to generate sum part: {:?}",
        res.err()
    );

    let res = BatchAggregator::new(
        &aggregation_name,
        &start_date,
        &end_date,
        false,
        &mut facilitator_ingest_transport,
        &mut facilitator_validate_transport,
        &mut pha_validate_transport,
        &mut aggregation_transport,
        &ingestor_pub_key,
        &facilitator_signing_key,
        &pha_pub_signing_key,
        &facilitator_ecies_key,
    )
    .unwrap()
    .generate_sum_part(&batch_ids_and_dates);
    assert!(
        res.is_ok(),
        "facilitator failed to generate sum part: {:?}",
        res.err()
    );

    let pha_aggregation_batch_reader: BatchReader<'_, SumPart, IngestionDataSharePacket> =
        BatchReader::new(
            Batch::new_sum(&aggregation_name, &start_date, &end_date, true),
            &mut aggregation_transport,
        );
    let pha_sum_part = pha_aggregation_batch_reader.header(&pha_pub_signing_key);
    assert!(
        pha_sum_part.is_ok(),
        "failed to read PHA sum part; {:?}",
        pha_sum_part.err()
    );
    let pha_sum_part = pha_sum_part.unwrap();
    let pha_sum_fields = pha_sum_part.sum().unwrap();

    let pha_invalid_packet_reader = pha_aggregation_batch_reader.packet_file_reader(&pha_sum_part);
    assert!(
        pha_invalid_packet_reader.is_err(),
        "should get no invalid packet reader when all packets were OK"
    );

    let facilitator_aggregation_batch_reader: BatchReader<'_, SumPart, IngestionDataSharePacket> =
        BatchReader::new(
            Batch::new_sum(&aggregation_name, &start_date, &end_date, false),
            &mut aggregation_transport,
        );
    let facilitator_sum_part =
        facilitator_aggregation_batch_reader.header(&facilitator_pub_signing_key);
    assert!(
        facilitator_sum_part.is_ok(),
        "failed to read PHA sum part; {:?}",
        facilitator_sum_part.err()
    );
    let facilitator_sum_part = facilitator_sum_part.unwrap();
    let facilitator_sum_fields = facilitator_sum_part.sum().unwrap();

    let facilitator_invalid_packet_reader =
        facilitator_aggregation_batch_reader.packet_file_reader(&facilitator_sum_part);
    assert!(
        facilitator_invalid_packet_reader.is_err(),
        "should get no invalid packet reader when all packets were OK"
    );

    let reconstructed = reconstruct_shares(&facilitator_sum_fields, &pha_sum_fields).unwrap();

    let reference_sum = reconstruct_shares(
        &batch_1_reference_sum.unwrap(),
        &batch_2_reference_sum.unwrap(),
    )
    .unwrap();
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
        reconstructed.len() as i64, facilitator_sum_part.total_individual_clients,
        "Total individual clients does not match the length of sum\n\
        \ttotal individual clients: {}\n\tlength of sum: {}",
        facilitator_sum_part.total_individual_clients, reconstructed.len()
    );
}
