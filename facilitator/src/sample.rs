use crate::{
    batch::{Batch, BatchWriter},
    idl::{IngestionDataSharePacket, IngestionHeader},
    logging::event,
    transport::SignableTransport,
    DATE_FORMAT,
};
use anyhow::{anyhow, Context, Result};
use bitvec::prelude::*;
use chrono::NaiveDateTime;
use prio::{
    client::Client,
    encrypt::PublicKey,
    field::{FieldElement, FieldPriov2},
};
use ring::digest;
use slog::{info, o, Logger};
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

impl SampleOutput {
    /// Returns true if the count-th packet should be omitted given the provided
    /// drop_nth_packet value.
    /// This should just be a method on SampleOutput but we use an associated
    /// function to work around an oddity with closures borrowing parts of a
    /// struct: https://github.com/rust-lang/rust/issues/53488
    fn drop_packet(drop_nth_packet: Option<usize>, count: usize) -> bool {
        matches!(drop_nth_packet, Some(nth) if count % nth == 0)
    }
}

/// The reference sum from a generated sample, along with metadata about the
/// generated sample.
#[derive(Debug)]
pub struct ReferenceSum {
    /// The reference sum, covering those packets whose shares appear in both
    /// PHA and facilitator ingestion batches.
    pub sum: Vec<FieldPriov2>,
    /// The number of contributions that went into the reference sum.
    pub contributions: usize,
    /// UUIDs of PHA packets that were dropped
    pub pha_dropped_packets: Vec<Uuid>,
    /// UUIDs of facilitator packets that were dropped
    pub facilitator_dropped_packets: Vec<Uuid>,
}

/// SampleGenerator constructs random data and splits it into two shares which
/// may be processed by data share processors. It allows tampering with
/// generated data to support various test cases.
#[derive(Debug)]
pub struct SampleGenerator<'a> {
    /// The name of the aggregation
    aggregation_name: &'a str,
    /// The dimension of the vector to be generated.
    dimension: i32,
    /// The differential privacy parameter applied to the data. Since the data
    /// generated by SampleGenerator is random, this value is ignored, except
    /// that it will be encoded into the generated batch's header.
    epsilon: f64,
    /// The start time for this batch, encoded as seconds since the start of the
    /// UNIX epoch. This value is ignored, except that it will be encoded into
    /// the generated batch's header.
    batch_start_time: i64,
    /// The end time for this batch, encoded as seconds in the start of the UNIX
    /// epoch. This value is ignored, except that it will be encoded into the
    /// generated batch's header.
    batch_end_time: i64,
    /// If this is Some(n), then when generating the nth packet,
    /// generate_ingestion_sample will generate data with a smaller dimension
    /// than the rest, such that on the server end, decryption will succeed, but
    /// deserialization and proof unpacking will fail. This is intended for
    /// testing.
    generate_short_packet: Option<usize>,
    /// Describes where the PHA/"first" server's shares should be written and
    /// how
    pha_output: &'a SampleOutput,
    /// Describes where the facilitator/"second" server's shares should be
    /// written and how
    facilitator_output: &'a SampleOutput,
    /// Logger to which events will be written
    logger: Logger,
}

impl<'a> SampleGenerator<'a> {
    /// Creates a new SampleGenerator. See the documentation on struct
    /// SampleGenerator for discussion of each parameter.
    pub fn new(
        aggregation_name: &'a str,
        dimension: i32,
        epsilon: f64,
        batch_start_time: i64,
        batch_end_time: i64,
        pha_output: &'a SampleOutput,
        facilitator_output: &'a SampleOutput,
        parent_logger: &Logger,
    ) -> Self {
        let logger = parent_logger.new(o!(
            event::AGGREGATION_NAME => aggregation_name.to_owned(),
        ));
        Self {
            aggregation_name,
            dimension,
            epsilon,
            batch_start_time,
            batch_end_time,
            generate_short_packet: None,
            pha_output,
            facilitator_output,
            logger,
        }
    }

    /// Returns true if the count-th packet should be generated with the wrong
    /// dimension.
    /// This should just be a method on SampleGenerator but we use an associated
    /// function to work around an oddity with closures borrowing parts of a
    /// struct: https://github.com/rust-lang/rust/issues/53488
    fn short_packet(generate_short_packet: Option<usize>, count: usize) -> bool {
        matches!(generate_short_packet, Some(nth) if count == nth)
    }

    /// When generating the count-th packet, generate_ingestion_sample will
    /// generate data with a smaller dimension than the rest, such that on the
    /// server end, decryption will succeed, but deserialization and proof
    /// unpacking wil fail. This is intended for testing.
    pub fn set_generate_short_packet(&mut self, count: usize) {
        self.generate_short_packet = Some(count);
    }

    /// Generate random sample data, split it into shares, and transmit it to
    /// facilitator servers.
    ///
    /// The provided `batch_uuid` and `date` are used to construct filenames.
    /// `packet_count` packets are generated.
    /// The PHA/"first" server's shares are written to `pha_output`, and the
    /// facilitator/"second" server's shares are written to
    /// `facilitator_output`.
    ///
    /// Returns a `ReferenceSum` containing the sum over the unshared data.
    pub fn generate_ingestion_sample(
        &self,
        trace_id: &Uuid,
        batch_uuid: &Uuid,
        date: &NaiveDateTime,
        packet_count: usize,
    ) -> Result<ReferenceSum> {
        let local_logger = self.logger.new(o!(
            event::TRACE_ID => trace_id.to_string(),
            event::BATCH_ID => batch_uuid.to_string(),
            event::BATCH_DATE => date.format(DATE_FORMAT).to_string(),
            "pha_output_path" => self.pha_output.transport.transport.path(),
            "facilitator_output_path" => self.facilitator_output.transport.transport.path(),
        ));

        info!(self.logger, "Starting a sample generation job.");
        if self.dimension <= 0 {
            return Err(anyhow!("dimension must be an integer greater than zero"));
        }

        let pha_ingestion_batch = BatchWriter::new(
            Batch::new_ingestion(self.aggregation_name, batch_uuid, date),
            &*self.pha_output.transport.transport,
            trace_id,
        );
        let facilitator_ingestion_batch = BatchWriter::new(
            Batch::new_ingestion(self.aggregation_name, batch_uuid, date),
            &*self.facilitator_output.transport.transport,
            trace_id,
        );

        // Generate random data packets and write into data share packets
        let mut client = Client::new(
            // usize is probably bigger than i32 and we have checked that dim is
            // positive so this is safe
            self.dimension as usize,
            self.pha_output.packet_encryption_public_key.clone(),
            self.facilitator_output.packet_encryption_public_key.clone(),
        )
        .context("failed to create client (bad dimension parameter?)")?;

        let mut short_packet_client = Client::new(
            (self.dimension - 1) as usize,
            self.pha_output.packet_encryption_public_key.clone(),
            self.facilitator_output.packet_encryption_public_key.clone(),
        )
        .context("failed to create client (bad dimension parameter?)")?;

        // Borrowing distinct parts of a struct like the SampleOutputs works, but
        // not under closures: https://github.com/rust-lang/rust/issues/53488
        // The workaround is to borrow or copy fields outside the closure.
        let drop_nth_pha_packet = self.pha_output.drop_nth_packet;
        let drop_nth_facilitator_packet = self.facilitator_output.drop_nth_packet;
        let generate_short_packet = self.generate_short_packet;
        let dimension = self.dimension;
        let aggregation_name = self.aggregation_name;
        let epsilon = self.epsilon;
        let batch_start_time = self.batch_start_time;
        let batch_end_time = self.batch_end_time;

        let mut reference_sum = vec![FieldPriov2::from(0); self.dimension as usize];
        let mut contributions = 0;
        let mut pha_packets = Vec::new();
        let mut pha_dropped_packets = Vec::new();
        let mut facilitator_packets = Vec::new();
        let mut facilitator_dropped_packets = Vec::new();

        // Compute packets & dropped packets for facilitator & PHA.
        for (count, mut data) in sample_data_iterator(batch_uuid, dimension as usize)
            .take(packet_count)
            .enumerate()
        {
            // Shorten the returned data vector if requested.
            if Self::short_packet(generate_short_packet, count) {
                data.pop();
            }

            // Update reference sum. (If we are dropping the packet from either
            // output, do not include it in the reference sum.)
            if !SampleOutput::drop_packet(drop_nth_pha_packet, count)
                && !SampleOutput::drop_packet(drop_nth_facilitator_packet, count)
            {
                for (r, d) in reference_sum.iter_mut().zip(data.iter()) {
                    *r += *d
                }
                contributions += 1;
            }

            let curr_client = if Self::short_packet(generate_short_packet, count) {
                &mut short_packet_client
            } else {
                &mut client
            };

            let (pha_share, facilitator_share) = curr_client
                .encode_simple(&data)
                .context("failed to encode data")?;

            // Hardcoded r_pit value
            // This value can be dynamic by running an instance of libprio::Server
            // However, libprio::Server takes in a private key for initialization
            // which we don't have in this context. Using a constant value removes
            // the libprio::Server dependency for creating samples
            let r_pit: u32 = 998314904;
            let packet_uuid = Uuid::new_v4();

            if SampleOutput::drop_packet(drop_nth_pha_packet, count) {
                info!(
                    local_logger,
                    "dropping packet #{} {} from PHA ingestion batch", count, packet_uuid
                );
                pha_dropped_packets.push(packet_uuid);
            } else {
                pha_packets.push(IngestionDataSharePacket {
                    uuid: packet_uuid,
                    encrypted_payload: pha_share,
                    encryption_key_id: Some("pha-fake-key-1".to_owned()),
                    r_pit: r_pit as i64,
                    version_configuration: Some("config-1".to_owned()),
                    device_nonce: None,
                });
            }

            if SampleOutput::drop_packet(drop_nth_facilitator_packet, count) {
                info!(
                    local_logger,
                    "dropping packet #{} {} from facilitator ingestion batch", count, packet_uuid
                );
                facilitator_dropped_packets.push(packet_uuid);
            } else {
                facilitator_packets.push(IngestionDataSharePacket {
                    uuid: packet_uuid,
                    encrypted_payload: facilitator_share,
                    encryption_key_id: None,
                    r_pit: r_pit as i64,
                    version_configuration: Some("config-1".to_owned()),
                    device_nonce: None,
                });
            }
        }

        // Write facilitator & PHA batches.
        facilitator_ingestion_batch.write(
            &self.facilitator_output.transport.batch_signing_key,
            IngestionHeader {
                batch_uuid: *batch_uuid,
                name: aggregation_name.to_owned(),
                bins: dimension,
                epsilon,
                prime: FieldPriov2::modulus() as i64,
                number_of_servers: 2,
                hamming_weight: None,
                batch_start_time,
                batch_end_time,
                packet_file_digest: Vec::new(),
            },
            facilitator_packets,
        )?;
        pha_ingestion_batch.write(
            &self.pha_output.transport.batch_signing_key,
            IngestionHeader {
                batch_uuid: *batch_uuid,
                name: aggregation_name.to_owned(),
                bins: dimension,
                epsilon,
                prime: FieldPriov2::modulus() as i64,
                number_of_servers: 2,
                hamming_weight: None,
                batch_start_time,
                batch_end_time,
                packet_file_digest: Vec::new(),
            },
            pha_packets,
        )?;

        info!(local_logger, "done");
        Ok(ReferenceSum {
            sum: reference_sum,
            contributions,
            pha_dropped_packets,
            facilitator_dropped_packets,
        })
    }
}

/// Generates the expected sum for a given sum batch that was generated based
/// off of sample ingestion batches (i.e. by `generate_ingestion_sample`).
/// Typically, `batch_uuids` will come directly from SumPart.batch_uuids,
/// `dimension` will come directly from SumPart.bins, and `packet_count` must
/// match the `packet_count` that was used in the calls to
/// `generate_ingestion_sample`. It is expected that neither the "short packet"
/// nor the "drop packet" features were used during sample generation.
pub fn expected_sample_sum_for_batches(
    batch_uuids: &[Uuid],
    dimension: i32,
    packet_count: usize,
) -> Vec<FieldPriov2> {
    let dimension = dimension as usize;
    let mut sum = vec![FieldPriov2::from(0); dimension];

    for batch_uuid in batch_uuids {
        for packet_data in sample_data_iterator(batch_uuid, dimension).take(packet_count) {
            assert_eq!(sum.len(), packet_data.len());
            for (s, v) in sum.iter_mut().zip(packet_data.iter()) {
                *s += *v;
            }
        }
    }

    sum
}

/// Returns an iterator over sample data; each datum will be a vector of length
/// `dimension`. This iterator will produce an arbitrarily large number of
/// values, with the expectation that the caller will terminate iteration when
/// enough values have been taken. The sequence of output values is
/// deterministic based on `batch_uuid` & `dimension`.
fn sample_data_iterator(
    batch_uuid: &Uuid,
    dimension: usize,
) -> impl Iterator<Item = Vec<FieldPriov2>> {
    // Ensure preconditions.
    static DIGEST_ALGORITHM: &digest::Algorithm = &digest::SHA512_256;
    assert!(
        dimension <= 8 * DIGEST_ALGORITHM.output_len,
        "Requested dimension {} is too large for digest algorithm in use (maximum dimension {})",
        dimension,
        8 * DIGEST_ALGORITHM.output_len
    );

    let batch_uuid = batch_uuid.to_owned();
    (0_u64..).map(move |index| {
        // Compute a digest over (batch_uuid, index) to produce deterministic, arbitrary,
        // unique bits for data.
        let mut ctx = digest::Context::new(DIGEST_ALGORITHM);
        ctx.update(batch_uuid.as_bytes());
        ctx.update(&index.to_be_bytes());
        let digest = ctx.finish();

        // Generate data based on the computed digest.
        digest
            .as_bits::<Msb0>()
            .iter()
            .take(dimension)
            .map(|bit| FieldPriov2::from(*bit as u32))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::iter;

    use super::*;
    use crate::{
        idl::Header,
        logging::setup_test_logging,
        test_utils::{
            default_facilitator_packet_encryption_public_key,
            default_facilitator_signing_private_key, default_ingestor_private_key,
            default_pha_packet_encryption_public_key, default_pha_signing_private_key,
            DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY, DEFAULT_PHA_ECIES_PRIVATE_KEY, DEFAULT_TRACE_ID,
        },
        transport::{LocalFileTransport, Transport},
    };
    use chrono::NaiveDate;
    use prio::encrypt::PrivateKey;
    use tempfile::TempDir;

    #[test]
    #[allow(clippy::float_cmp)] // No arithmetic done on floats
    fn write_sample() {
        let logger = setup_test_logging();
        let tempdir = tempfile::TempDir::new().unwrap();
        let batch_uuid = Uuid::new_v4();

        let pha_output = SampleOutput {
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
        let facilitator_output = SampleOutput {
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

        let sample_generator = SampleGenerator::new(
            "fake-aggregation",
            10,
            0.11,
            100,
            100,
            &pha_output,
            &facilitator_output,
            &logger,
        );

        sample_generator
            .generate_ingestion_sample(
                &DEFAULT_TRACE_ID,
                &batch_uuid,
                &NaiveDate::from_ymd_opt(2009, 2, 13)
                    .unwrap()
                    .and_hms_opt(23, 31, 0)
                    .unwrap(),
                10,
            )
            .unwrap();
        let expected_path = format!("fake-aggregation/2009/02/13/23/31/{batch_uuid}.batch");

        let transports = &mut [
            LocalFileTransport::new(tempdir.path().to_path_buf().join("pha")),
            LocalFileTransport::new(tempdir.path().to_path_buf().join("facilitator")),
        ];
        for transport in transports {
            let reader = transport.get(&expected_path, &DEFAULT_TRACE_ID).unwrap();

            let parsed_header = IngestionHeader::read(reader).unwrap();
            assert_eq!(parsed_header.batch_uuid, batch_uuid);
            assert_eq!(parsed_header.name, "fake-aggregation".to_owned());
            assert_eq!(parsed_header.bins, 10);
            assert_eq!(parsed_header.epsilon, 0.11);
            assert_eq!(parsed_header.prime, FieldPriov2::modulus() as i64);
            assert_eq!(parsed_header.number_of_servers, 2);
            assert_eq!(parsed_header.hamming_weight, None);
            assert_eq!(parsed_header.batch_start_time, 100);
            assert_eq!(parsed_header.batch_end_time, 100);
        }
    }

    #[test]
    fn expected_sample_sum_for_batches() {
        const BATCH_COUNT: usize = 4;
        const DIMENSION: i32 = 123;
        const PACKET_COUNT: usize = 10;

        let logger = setup_test_logging();
        let tempdir = TempDir::new().unwrap();
        let batch_uuids: Vec<Uuid> = iter::repeat_with(Uuid::new_v4).take(BATCH_COUNT).collect();

        // Determine expected sum, per expected_sample_sum_for_batches.
        let expected_sum =
            super::expected_sample_sum_for_batches(&batch_uuids, DIMENSION, PACKET_COUNT);

        // Create SampleGenerator, along with its dependencies.
        let sample_output_pha = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    tempdir.path().to_path_buf().join("pha"),
                )),
                batch_signing_key: default_pha_signing_private_key(),
            },
            packet_encryption_public_key: default_pha_packet_encryption_public_key(),
            drop_nth_packet: None,
        };

        let sample_output_facilitator = SampleOutput {
            transport: SignableTransport {
                transport: Box::new(LocalFileTransport::new(
                    tempdir.path().to_path_buf().join("facilitator"),
                )),
                batch_signing_key: default_facilitator_signing_private_key(),
            },
            packet_encryption_public_key: default_facilitator_packet_encryption_public_key(),
            drop_nth_packet: None,
        };

        let sample_generator = SampleGenerator::new(
            "fake-aggregation",
            DIMENSION,
            0.0,
            0,
            0,
            &sample_output_pha,
            &sample_output_facilitator,
            &logger,
        );

        // Determine actual sum, per calls to generate_ingestion_sample.
        let mut actual_sum = vec![FieldPriov2::from(0); DIMENSION as usize];
        for batch_uuid in batch_uuids {
            let batch_sum = sample_generator
                .generate_ingestion_sample(
                    &DEFAULT_TRACE_ID,
                    &batch_uuid,
                    &NaiveDate::from_ymd_opt(2009, 2, 13)
                        .unwrap()
                        .and_hms_opt(23, 31, 0)
                        .unwrap(),
                    PACKET_COUNT,
                )
                .unwrap()
                .sum;
            assert_eq!(batch_sum.len(), actual_sum.len());
            for (s, v) in actual_sum.iter_mut().zip(batch_sum.iter()) {
                *s += *v;
            }
        }

        // Check that the expected sum matches the actual sum.
        assert_eq!(expected_sum, actual_sum);
    }
}
