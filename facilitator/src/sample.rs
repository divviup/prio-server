use crate::{
    batch::{Batch, BatchWriter},
    idl::{IngestionDataSharePacket, IngestionHeader, Packet},
    transport::SignableTransport,
};
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDateTime;
use log::info;
use prio::{
    client::Client,
    encrypt::PublicKey,
    finite_field::{Field, MODULUS},
};
use rand::{thread_rng, Rng};
use uuid::Uuid;

/// Configuration for output from sample generation.
#[derive(Debug)]
pub struct SampleOutput {
    /// SignableTransport to which to write the ingestion batch
    pub transport: SignableTransport,
    /// Encryption key with which to encrypt ingestion shares
    pub packet_encryption_public_key: PublicKey,
    /// If this is Some(n), then generate_ingestion_sample will omit every nth
    /// packet generated from the ingestion batch sent to this output. This is
    /// intended for testing.
    pub drop_nth_packet: Option<usize>,
}

#[derive(Debug)]
pub struct ReferenceSum {
    /// The reference sum, covering those packets whose shares appear in both
    /// PHA and facilitator ingestion batches.
    pub sum: Vec<Field>,
    /// The number of contributions that went into the reference sum.
    pub contributions: usize,
    /// UUIDs of PHA packets that were dropped
    pub pha_dropped_packets: Vec<Uuid>,
    /// UUIDs of facilitator packets that were dropped
    pub facilitator_dropped_packets: Vec<Uuid>,
}

fn drop_packet(count: usize, drop_nth_packet: Option<usize>) -> bool {
    match drop_nth_packet {
        Some(nth) if count % nth == 0 => true,
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)] // Grandfathered in
pub fn generate_ingestion_sample(
    batch_uuid: &Uuid,
    aggregation_name: &str,
    date: &NaiveDateTime,
    dim: i32,
    packet_count: usize,
    epsilon: f64,
    batch_start_time: i64,
    batch_end_time: i64,
    pha_output: &mut SampleOutput,
    facilitator_output: &mut SampleOutput,
) -> Result<ReferenceSum> {
    if dim <= 0 {
        return Err(anyhow!("dimension must be an integer greater than zero"));
    }

    let mut pha_ingestion_batch: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
        BatchWriter::new(
            Batch::new_ingestion(aggregation_name, batch_uuid, date),
            &mut *pha_output.transport.transport,
        );
    let mut facilitator_ingestion_batch: BatchWriter<
        '_,
        IngestionHeader,
        IngestionDataSharePacket,
    > = BatchWriter::new(
        Batch::new_ingestion(aggregation_name, batch_uuid, date),
        &mut *facilitator_output.transport.transport,
    );

    // Generate random data packets and write into data share packets
    let mut thread_rng = thread_rng();

    let mut client = Client::new(
        // usize is probably bigger than i32 and we have checked that dim is
        // positive so this is safe
        dim as usize,
        pha_output.packet_encryption_public_key.clone(),
        facilitator_output.packet_encryption_public_key.clone(),
    )
    .context("failed to create client (bad dimension parameter?)")?;

    // Borrowing distinct parts of a struct like the SampleOutputs works, but
    // not under closures: https://github.com/rust-lang/rust/issues/53488
    // The workaround is to borrow or copy fields outside the closure.
    let facilitator_batch_signing_key_ref = &facilitator_output.transport.batch_signing_key;
    let drop_nth_pha_packet = pha_output.drop_nth_packet;
    let drop_nth_facilitator_packet = facilitator_output.drop_nth_packet;

    let mut reference_sum = vec![Field::from(0); dim as usize];
    let mut contributions = 0;
    let mut pha_dropped_packets = Vec::new();
    let mut facilitator_dropped_packets = Vec::new();

    // We nest the closures here to get both packet writers in one scope
    let pha_packet_file_digest =
        pha_ingestion_batch.packet_file_writer(|mut pha_packet_writer| {
            let facilitator_packet_file_digest = facilitator_ingestion_batch.packet_file_writer(
                |mut facilitator_packet_writer| {
                    for count in 0..packet_count {
                        // Generate random bit vector
                        let data = (0..dim)
                            .map(|_| Field::from(thread_rng.gen_range(0, 2)))
                            .collect::<Vec<Field>>();

                        // If we are dropping the packet from either output, do
                        // not include it in the reference sum
                        if !drop_packet(count, drop_nth_pha_packet)
                            && !drop_packet(count, drop_nth_facilitator_packet)
                        {
                            for (r, d) in reference_sum.iter_mut().zip(data.iter()) {
                                *r += *d
                            }
                            contributions += 1;
                        }

                        let (pha_share, facilitator_share) = client
                            .encode_simple(&data)
                            .context("failed to encode data")?;

                        // Hardcoded r_pit value
                        // This value can be dynamic by running an instance of libprio::Server
                        // However, libprio::Server takes in a private key for initialization
                        // which we don't have in this context. Using a constant value removes
                        // the libprio::Server dependency for creating samples
                        let r_pit: u32 = 998314904;
                        let packet_uuid = Uuid::new_v4();

                        let pha_packet = IngestionDataSharePacket {
                            uuid: packet_uuid,
                            encrypted_payload: pha_share,
                            encryption_key_id: Some("pha-fake-key-1".to_owned()),
                            r_pit: u32::from(r_pit) as i64,
                            version_configuration: Some("config-1".to_owned()),
                            device_nonce: None,
                        };

                        if drop_packet(count, drop_nth_pha_packet) {
                            info!(
                                "dropping packet #{} {} from PHA ingestion batch",
                                count, packet_uuid
                            );
                            pha_dropped_packets.push(packet_uuid);
                        } else {
                            pha_packet.write(&mut pha_packet_writer)?;
                        }

                        let facilitator_packet = IngestionDataSharePacket {
                            uuid: packet_uuid,
                            encrypted_payload: facilitator_share,
                            encryption_key_id: None,
                            r_pit: u32::from(r_pit) as i64,
                            version_configuration: Some("config-1".to_owned()),
                            device_nonce: None,
                        };

                        if drop_packet(count, drop_nth_facilitator_packet) {
                            info!(
                                "dropping packet #{} {} from facilitator ingestion batch",
                                count, packet_uuid
                            );
                            facilitator_dropped_packets.push(packet_uuid);
                        } else {
                            facilitator_packet.write(&mut facilitator_packet_writer)?;
                        }
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
                &facilitator_batch_signing_key_ref.key,
            )?;

            facilitator_ingestion_batch.put_signature(
                &facilitator_header_signature,
                &facilitator_batch_signing_key_ref.identifier,
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
        &pha_output.transport.batch_signing_key.key,
    )?;
    pha_ingestion_batch.put_signature(
        &pha_header_signature,
        &pha_output.transport.batch_signing_key.identifier,
    )?;
    println!("done");
    Ok(ReferenceSum {
        sum: reference_sum,
        contributions,
        pha_dropped_packets,
        facilitator_dropped_packets,
    })
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
        transport::{LocalFileTransport, Transport},
    };
    use chrono::NaiveDate;

    #[test]
    #[allow(clippy::float_cmp)] // No arithmetic done on floats
    fn write_sample() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let batch_uuid = Uuid::new_v4();

        let mut pha_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    tempdir.path().to_path_buf().join("pha"),
                )),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from(
                &PrivateKey::from_base64(DEFAULT_PHA_ECIES_PRIVATE_KEY).unwrap(),
            ),
            drop_nth_packet: None,
        };
        let mut facilitator_output = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    tempdir.path().to_path_buf().join("facilitator"),
                )),
                batch_signing_key: default_ingestor_private_key(),
            },
            packet_encryption_public_key: PublicKey::from(
                &PrivateKey::from_base64(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY).unwrap(),
            ),
            drop_nth_packet: None,
        };

        let res = generate_ingestion_sample(
            &batch_uuid,
            "fake-aggregation",
            &NaiveDate::from_ymd(2009, 2, 13).and_hms(23, 31, 0),
            10,
            10,
            0.11,
            100,
            100,
            &mut pha_output,
            &mut facilitator_output,
        );
        assert!(res.is_ok(), "error writing sample data {:?}", res.err());
        let expected_path = format!("fake-aggregation/2009/02/13/23/31/{}.batch", batch_uuid);

        let transports = &mut [
            LocalFileTransport::new(tempdir.path().to_path_buf().join("pha")),
            LocalFileTransport::new(tempdir.path().to_path_buf().join("facilitator")),
        ];
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
