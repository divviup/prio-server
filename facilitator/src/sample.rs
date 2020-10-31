use crate::{
    batch::{Batch, BatchWriter},
    idl::{IngestionDataSharePacket, IngestionHeader, Packet},
    transport::Transport,
    BatchSigningKey,
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
use uuid::Uuid;

#[allow(clippy::too_many_arguments)] // Grandfathered in
pub fn generate_ingestion_sample(
    pha_transport: &mut dyn Transport,
    facilitator_transport: &mut dyn Transport,
    batch_uuid: &Uuid,
    aggregation_name: &str,
    date: &NaiveDateTime,
    pha_key: &PrivateKey,
    facilitator_key: &PrivateKey,
    ingestor_signing_key: &BatchSigningKey,
    dim: i32,
    packet_count: usize,
    epsilon: f64,
    batch_start_time: i64,
    batch_end_time: i64,
) -> Result<Vec<Field>> {
    if dim <= 0 {
        return Err(anyhow!("dimension must be an integer greater than zero"));
    }

    let mut pha_ingestion_batch: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
        BatchWriter::new(
            Batch::new_ingestion(aggregation_name, batch_uuid, date),
            pha_transport,
        );
    let mut facilitator_ingestion_batch: BatchWriter<
        '_,
        IngestionHeader,
        IngestionDataSharePacket,
    > = BatchWriter::new(
        Batch::new_ingestion(aggregation_name, batch_uuid, date),
        facilitator_transport,
    );

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
                            encryption_key_id: Some("pha-fake-key-1".to_owned()),
                            r_pit: u32::from(r_pit) as i64,
                            version_configuration: Some("config-1".to_owned()),
                            device_nonce: None,
                        };

                        pha_packet.write(&mut pha_packet_writer)?;

                        let facilitator_packet = IngestionDataSharePacket {
                            uuid: packet_uuid,
                            encrypted_payload: facilitator_share,
                            encryption_key_id: None,
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
                    batch_uuid: *batch_uuid,
                    name: aggregation_name.to_owned(),
                    bins: dim,
                    epsilon,
                    prime: MODULUS as i64,
                    number_of_servers: 2,
                    hamming_weight: None,
                    batch_start_time,
                    batch_end_time,
                    packet_file_digest: facilitator_packet_file_digest.as_ref().to_vec(),
                },
                &ingestor_signing_key.key,
            )?;

            facilitator_ingestion_batch.put_signature(
                &facilitator_header_signature,
                &ingestor_signing_key.identifier,
            )
        })?;

    let pha_header_signature = pha_ingestion_batch.put_header(
        &IngestionHeader {
            batch_uuid: *batch_uuid,
            name: aggregation_name.to_owned(),
            bins: dim,
            epsilon,
            prime: MODULUS as i64,
            number_of_servers: 2,
            hamming_weight: None,
            batch_start_time,
            batch_end_time,
            packet_file_digest: pha_packet_file_digest.as_ref().to_vec(),
        },
        &ingestor_signing_key.key,
    )?;
    pha_ingestion_batch.put_signature(&pha_header_signature, &ingestor_signing_key.identifier)?;
    println!("done");
    Ok(reference_sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        idl::Header,
        test_utils::{
            default_ingestor_private_key, DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY,
            DEFAULT_PHA_ECIES_PRIVATE_KEY,
        },
        transport::LocalFileTransport,
    };
    use chrono::NaiveDate;

    #[test]
    #[allow(clippy::float_cmp)] // No arithmetic done on floats
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
            &NaiveDate::from_ymd(2009, 2, 13).and_hms(23, 31, 0),
            &PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            &PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
            &default_ingestor_private_key(),
            10,
            10,
            0.11,
            100,
            100,
        );
        assert!(res.is_ok(), "error writing sample data {:?}", res.err());
        let expected_path = format!("fake-aggregation/2009/02/13/23/31/{}.batch", batch_uuid);

        let transports = &mut [pha_transport, facilitator_transport];
        for transport in transports {
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
