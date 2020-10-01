use crate::{
    batch::{BatchIO, BatchWriter},
    idl::{IngestionDataSharePacket, IngestionHeader, Packet},
    transport::Transport,
};
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDateTime;
use prio::{
    client::Client,
    encrypt::{PrivateKey, PublicKey},
    finite_field::{Field, MODULUS},
    server::Server,
};
use rand::{thread_rng, Rng};
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use uuid::Uuid;

pub fn generate_ingestion_sample(
    pha_transport: &mut dyn Transport,
    facilitator_transport: &mut dyn Transport,
    batch_uuid: &Uuid,
    aggregation_name: &str,
    date: &NaiveDateTime,
    pha_key: &PrivateKey,
    facilitator_key: &PrivateKey,
    ingestor_key: &[u8],
    dim: i32,
    packet_count: usize,
    epsilon: f64,
    batch_start_time: i64,
    batch_end_time: i64,
) -> Result<Vec<Field>> {
    if dim <= 0 {
        return Err(anyhow!("dimension must be an integer greater than zero"));
    }

    let ingestor_key_pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, ingestor_key)
            .context("failed to parse ingestor key pair")?;

    let mut pha_ingestion_batch: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
        BatchWriter::new_ingestion(aggregation_name, batch_uuid, date, pha_transport)?;
    let mut facilitator_ingestion_batch: BatchWriter<
        '_,
        IngestionHeader,
        IngestionDataSharePacket,
    > = BatchWriter::new_ingestion(aggregation_name, batch_uuid, date, facilitator_transport)?;

    // Generate random data packets and write into data share packets
    let mut thread_rng = thread_rng();

    let mut client = Client::new(
        // usize is probably bigger than i32 and we have checked that dim is
        // positive so this is safe
        dim as usize,
        PublicKey::from(pha_key),
        PublicKey::from(facilitator_key),
    )
    .context("failed to create client (bad dimension parameter?)")?;

    let mut reference_sum = vec![Field::from(0); dim as usize];

    // We nest the closures here to get both packet writers in one scope
    let pha_packet_file_digest =
        pha_ingestion_batch.packet_file_writer(|mut pha_packet_writer| {
            let facilitator_packet_file_digest = facilitator_ingestion_batch.packet_file_writer(
                |mut facilitator_packet_writer| {
                    // We need an instance of a libprio server to pick an r_pit.
                    let fake_server = Server::new(dim as usize, true, pha_key.clone());

                    for _ in 0..packet_count {
                        // Generate random bit vector
                        let data = (0..dim)
                            .map(|_| Field::from(thread_rng.gen_range(0, 2)))
                            .collect::<Vec<Field>>();

                        for (r, d) in reference_sum.iter_mut().zip(data.iter()) {
                            *r += *d
                        }

                        let (pha_share, facilitator_share) = client
                            .encode_simple(&data)
                            .context("failed to encode data")?;

                        let r_pit = fake_server.choose_eval_at();
                        let packet_uuid = Uuid::new_v4();

                        let pha_packet = IngestionDataSharePacket {
                            uuid: packet_uuid,
                            encrypted_payload: pha_share,
                            encryption_key_id: "pha-fake-key-1".to_owned(),
                            r_pit: u32::from(r_pit) as i64,
                            version_configuration: Some("config-1".to_owned()),
                            device_nonce: None,
                        };

                        pha_packet.write(&mut pha_packet_writer)?;

                        let facilitator_packet = IngestionDataSharePacket {
                            uuid: packet_uuid,
                            encrypted_payload: facilitator_share,
                            encryption_key_id: "facilitator-fake-key-1".to_owned(),
                            r_pit: u32::from(r_pit) as i64,
                            version_configuration: Some("config-1".to_owned()),
                            device_nonce: None,
                        };

                        facilitator_packet.write(&mut facilitator_packet_writer)?;
                    }
                    Ok(())
                },
            )?;

            let facilitator_header_signature = facilitator_ingestion_batch.put_header(
                &IngestionHeader {
                    batch_uuid: batch_uuid.clone(),
                    name: aggregation_name.to_owned(),
                    bins: dim,
                    epsilon: epsilon,
                    prime: MODULUS as i64,
                    number_of_servers: 2,
                    hamming_weight: None,
                    batch_start_time: batch_start_time,
                    batch_end_time: batch_end_time,
                    packet_file_digest: facilitator_packet_file_digest.as_ref().to_vec(),
                },
                &ingestor_key_pair,
            )?;

            Ok(facilitator_ingestion_batch.put_signature(&facilitator_header_signature)?)
        })?;

    let pha_header_signature = pha_ingestion_batch.put_header(
        &IngestionHeader {
            batch_uuid: batch_uuid.clone(),
            name: aggregation_name.to_owned(),
            bins: dim,
            epsilon: epsilon,
            prime: MODULUS as i64,
            number_of_servers: 2,
            hamming_weight: None,
            batch_start_time: batch_start_time,
            batch_end_time: batch_end_time,
            packet_file_digest: pha_packet_file_digest.as_ref().to_vec(),
        },
        &ingestor_key_pair,
    )?;
    pha_ingestion_batch.put_signature(&pha_header_signature)?;
    Ok(reference_sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        idl::Header,
        test_utils::{
            default_ingestor_private_key_raw, DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY,
            DEFAULT_PHA_ECIES_PRIVATE_KEY,
        },
        transport::LocalFileTransport,
    };
    use std::path::PathBuf;

    #[test]
    #[ignore]
    fn write_sample() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let batch_uuid = Uuid::new_v4();
        let mut pha_transport = LocalFileTransport::new(tempdir.path().to_path_buf().join("pha"));
        let mut facilitator_transport =
            LocalFileTransport::new(tempdir.path().to_path_buf().join("pha"));

        let res = generate_ingestion_sample(
            &mut pha_transport,
            &mut facilitator_transport,
            &batch_uuid,
            "fake-aggregation",
            &NaiveDateTime::from_timestamp(1234567890, 654321),
            &PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            &PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
            &default_ingestor_private_key_raw(),
            10,
            10,
            0.11,
            100,
            100,
        );
        assert!(res.is_ok(), "error writing sample data {:?}", res.err());
        let mut expected_path =
            PathBuf::from("fake-aggregation/fake-date").join(batch_uuid.to_string());

        let transports = &[pha_transport, facilitator_transport];
        for transport in transports {
            expected_path.set_extension("batch");
            let reader = transport.get(&expected_path);
            assert!(res.is_ok(), "error reading header back {:?}", res.err());

            let parsed_header_res = IngestionHeader::read(reader.unwrap());
            assert!(
                parsed_header_res.is_ok(),
                "error parsing header {:?}",
                res.err()
            );
            let parsed_header = parsed_header_res.unwrap();
            assert_eq!(parsed_header.batch_uuid, batch_uuid);
            assert_eq!(parsed_header.name, "fake-aggregation".to_owned());
            assert_eq!(parsed_header.bins, 10);
            assert_eq!(parsed_header.epsilon, 0.11);
            assert_eq!(parsed_header.prime, MODULUS as i64);
            assert_eq!(parsed_header.number_of_servers, 2);
            assert_eq!(parsed_header.hamming_weight, None);
            assert_eq!(parsed_header.batch_start_time, 100);
            assert_eq!(parsed_header.batch_end_time, 100);
        }
    }
}
