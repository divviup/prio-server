use crate::{
    batch::{Batch, BatchReadError, BatchReader, BatchWriteError, BatchWriter},
    idl::{
        IngestionDataSharePacket, IngestionHeader, InvalidPacket, SumPart, ValidationHeader,
        ValidationPacket,
    },
    intake::IntakeError,
    logging::event,
    metrics::AggregateMetricsCollector,
    transport::{SignableTransport, VerifiableAndDecryptableTransport, VerifiableTransport},
    BatchSigningKey, ErrorClassification,
};
use chrono::NaiveDateTime;
use prio::{
    field::FieldPriov2,
    server::{Server, VerificationMessage},
};
use slog::{info, o, warn, Logger};
use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    num::TryFromIntError,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AggregationError {
    #[error("batch_ids_and_dates is empty")]
    EmptyBatchesAndDates,
    #[error(
        "ingestion header does not match peer validation header. Ingestion: {0:?}\nPeer:{1:?}"
    )]
    IngestionValidationHeaderMismatch(IngestionHeader, ValidationHeader),
    #[error("ingestion header parameters do not match. First ingestion header: {0:?}\nSecond ingestion header:{1:?}")]
    IngestionIngestionHeaderMismatch(IngestionHeader, IngestionHeader),
    #[error("error setting up Prio server: {0}")]
    PrioSetup(prio::server::ServerError),
    #[error("error accumulating shares: {0}")]
    PrioAccumulate(anyhow::Error),
    #[error("failed to validate packets: {0}")]
    Validation(anyhow::Error),
    #[error(transparent)]
    BatchRead(#[from] BatchReadError),
    #[error(transparent)]
    BatchWrite(#[from] BatchWriteError),
    #[error("intake error in aggregation context: {0}")]
    Intake(IntakeError),
    #[error("invalid verification message: {0}")]
    InvalidVerification(TryFromIntError),
}

impl ErrorClassification for AggregationError {
    fn is_retryable(&self) -> bool {
        match self {
            // If the batches and dates in this task are empty, retrying the task won't help.
            AggregationError::EmptyBatchesAndDates => false,
            // These indicate an issue with the peer server's validation message. Retries are not
            // necessary in this case.
            AggregationError::IngestionValidationHeaderMismatch(_, _)
            | AggregationError::IngestionIngestionHeaderMismatch(_, _)
            | AggregationError::InvalidVerification(_) => false,
            // libprio-rs errors may be due to getrandom failures.
            AggregationError::PrioSetup(_)
            | AggregationError::PrioAccumulate(_)
            | AggregationError::Validation(_) => true,
            // Dispatch to wrapped error types.
            AggregationError::BatchRead(e) => e.is_retryable(),
            AggregationError::BatchWrite(e) => e.is_retryable(),
            AggregationError::Intake(e) => e.is_retryable(),
        }
    }
}

pub struct BatchAggregator<'a> {
    trace_id: &'a Uuid,
    is_first: bool,
    permit_malformed_batch: bool,
    aggregation_name: &'a str,
    aggregation_start: &'a NaiveDateTime,
    aggregation_end: &'a NaiveDateTime,
    peer_validation_transport: &'a mut VerifiableTransport,
    ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
    aggregation_batch: BatchWriter<'a, SumPart, InvalidPacket>,
    share_processor_signing_key: &'a BatchSigningKey,
    total_individual_clients: i64,
    bytes_processed: u64,
    metrics_collector: Option<&'a AggregateMetricsCollector>,
    logger: Logger,
}

impl<'a> BatchAggregator<'a> {
    pub fn new(
        trace_id: &'a Uuid,
        instance_name: &str,
        aggregation_name: &'a str,
        aggregation_start: &'a NaiveDateTime,
        aggregation_end: &'a NaiveDateTime,
        is_first: bool,
        permit_malformed_batch: bool,
        ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
        peer_validation_transport: &'a mut VerifiableTransport,
        aggregation_transport: &'a mut SignableTransport,
        parent_logger: &Logger,
    ) -> Result<Self, AggregationError> {
        let logger = parent_logger.new(o!(
            event::TRACE_ID => trace_id.to_string(),
            event::AGGREGATION_NAME => aggregation_name.to_owned(),
            event::INGESTION_PATH => ingestion_transport.transport.transport.path(),
            event::PEER_VALIDATION_PATH => peer_validation_transport.transport.path(),
        ));
        Ok(Self {
            trace_id,
            is_first,
            permit_malformed_batch,
            aggregation_name,
            aggregation_start,
            aggregation_end,
            peer_validation_transport,
            ingestion_transport,
            aggregation_batch: BatchWriter::new(
                Batch::new_sum(
                    instance_name,
                    aggregation_name,
                    aggregation_start,
                    aggregation_end,
                    is_first,
                ),
                &*aggregation_transport.transport,
                trace_id,
            ),
            share_processor_signing_key: &aggregation_transport.batch_signing_key,
            total_individual_clients: 0,
            bytes_processed: 0,
            metrics_collector: None,
            logger,
        })
    }

    pub fn set_metrics_collector(&mut self, collector: &'a AggregateMetricsCollector) {
        self.metrics_collector = Some(collector);
    }

    /// Compute the sum part for all the provided batch IDs and write it out to
    /// the aggregation transport. The provided callback is invoked after each
    /// batch is aggregated.
    pub fn generate_sum_part<F>(
        &mut self,
        batch_ids_and_dates: &[(Uuid, NaiveDateTime)],
        mut callback: F,
    ) -> Result<(), AggregationError>
    where
        F: FnMut(&Logger),
    {
        info!(self.logger, "processing aggregation task");
        if batch_ids_and_dates.is_empty() {
            return Err(AggregationError::EmptyBatchesAndDates);
        }
        let mut invalid_uuids = Vec::new();
        let mut included_batch_uuids = Vec::new();

        let mut servers = None;
        let mut last_ingestion_header: Option<IngestionHeader> = None;
        for (batch_id, batch_date) in batch_ids_and_dates {
            let (ingestion_hdr, ingestion_packets) = BatchReader::new(
                Batch::new_ingestion(self.aggregation_name, batch_id, batch_date),
                &*self.ingestion_transport.transport.transport,
                self.permit_malformed_batch,
                self.trace_id,
                &self.logger,
            )
            .read(&self.ingestion_transport.transport.batch_signing_public_keys)?;

            if ingestion_packets.is_empty() {
                warn!(self.logger, "Skipping empty ingestion batch"; event::BATCH_ID => batch_id.to_string());
                continue;
            }

            let mut peer_validation_batch_reader = BatchReader::new(
                Batch::new_validation(self.aggregation_name, batch_id, batch_date, !self.is_first),
                &*self.peer_validation_transport.transport,
                self.permit_malformed_batch,
                self.trace_id,
                &self.logger,
            );
            if let Some(collector) = self.metrics_collector {
                peer_validation_batch_reader
                    .set_metrics_collector(&collector.peer_validation_batches_reader_metrics);
            }
            let (peer_validation_hdr, peer_validation_packets) = peer_validation_batch_reader
                .read(&self.peer_validation_transport.batch_signing_public_keys)?;

            if servers.is_none() {
                // Ideally, we would use the encryption_key_id in the ingestion packet
                // to figure out which private key to use for decryption, but that field
                // is optional. Instead we try all the keys we have available until one
                // works.
                // https://github.com/abetterinternet/prio-server/issues/73
                servers = Some(
                    self.ingestion_transport
                        .packet_decryption_keys
                        .iter()
                        .map(|k| Server::new(ingestion_hdr.bins as usize, self.is_first, k.clone()))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(AggregationError::PrioSetup)?,
                );
            }

            // Make sure all the parameters in the headers line up.
            if !ingestion_hdr.check_parameters_against_validation(&peer_validation_hdr) {
                return Err(AggregationError::IngestionValidationHeaderMismatch(
                    ingestion_hdr,
                    peer_validation_hdr,
                ));
            }
            if !last_ingestion_header
                .as_ref()
                .map(|h| h.check_parameters_against_ingestion(&ingestion_hdr))
                .unwrap_or(
                    true, /* no previous ingestion header means we pass validation */
                )
            {
                return Err(AggregationError::IngestionIngestionHeaderMismatch(
                    last_ingestion_header.unwrap(),
                    ingestion_hdr,
                ));
            }

            if self.aggregate_share(
                servers.as_mut().unwrap(),
                &mut invalid_uuids,
                ingestion_packets,
                peer_validation_packets,
            )? {
                // Include the batch UUID only if the batch was aggregated into
                // the sum part
                included_batch_uuids.push(batch_id.to_owned());
            }
            last_ingestion_header.get_or_insert(ingestion_hdr);
            callback(&self.logger);
        }
        let last_ingestion_header = last_ingestion_header.unwrap();

        if let Some(collector) = self.metrics_collector {
            collector
                .packets_per_sum_part
                .with_label_values(&[self.aggregation_name])
                .set(self.total_individual_clients);

            collector
                .packets_processed
                .with_label_values(&[self.aggregation_name])
                .inc_by(self.total_individual_clients as u64);

            collector
                .bytes_processed
                .with_label_values(&[self.aggregation_name])
                .inc_by(self.bytes_processed);
        }

        // We have one Server for each packet decryption key, and each of those
        // instances could contain some accumulated shares, depending on which
        // key was used to encrypt an individual packet. We make a new Server
        // instance into which we will aggregate them all together. It doesn't
        // matter which private key we use here as we're not decrypting any
        // packets with this Server instance, just accumulating data vectors.
        let mut accumulator_server = Server::new(
            last_ingestion_header.bins as usize,
            self.is_first,
            self.ingestion_transport.packet_decryption_keys[0].clone(),
        )
        .map_err(AggregationError::PrioSetup)?;
        for server in servers.unwrap().iter() {
            accumulator_server
                .merge_total_shares(server.total_shares())
                .map_err(|e| AggregationError::PrioAccumulate(e.into()))?;
        }
        let sum = accumulator_server
            .total_shares()
            .iter()
            .map(|f| u32::from(*f) as i64)
            .collect();

        // Write the aggregated data back to a sum part batch.
        self.aggregation_batch.write(
            self.share_processor_signing_key,
            SumPart {
                batch_uuids: included_batch_uuids,
                name: last_ingestion_header.name,
                bins: last_ingestion_header.bins,
                epsilon: last_ingestion_header.epsilon,
                prime: last_ingestion_header.prime,
                number_of_servers: last_ingestion_header.number_of_servers,
                hamming_weight: last_ingestion_header.hamming_weight,
                sum,
                aggregation_start_time: self.aggregation_start.timestamp_millis(),
                aggregation_end_time: self.aggregation_end.timestamp_millis(),
                packet_file_digest: Vec::new(),
                total_individual_clients: self.total_individual_clients,
            },
            invalid_uuids.into_iter().map(|u| InvalidPacket { uuid: u }),
        )?;

        Ok(())
    }

    /// Aggregate the batch for the provided batch_id into the provided server.
    /// The UUIDs of packets for which aggregation fails are recorded in the
    /// provided invalid_uuids vector. Returns `Ok(true)` if `ingestion_packets`
    /// were aggregated into the server, `Ok(false)` if `ingestion_packets` was
    /// rejected (in which case other batches in the aggregation window should
    /// still be summed) and `Err` if something went wrong (in which case the
    /// overall aggregation should be aborted).
    fn aggregate_share(
        &mut self,
        servers: &mut [Server<FieldPriov2>],
        invalid_uuids: &mut Vec<Uuid>,
        ingestion_packets: Vec<IngestionDataSharePacket>,
        peer_validation_packets: Vec<ValidationPacket>,
    ) -> Result<bool, AggregationError> {
        // We can't be sure that the peer validation, own validation and
        // ingestion batches contain all the same packets or that they are in
        // the same order. There could also be duplicate packets. Compared to
        // the ingestion packets, the validation packets are small (they only
        // contain a UUID and three i64s), so we construct a hashmap of the
        // validation batches, mapping packet UUID to a ValidationPacket, then
        // iterate over the ingestion packets. For each ingestion packet, if we
        // have the corresponding validation packets and the proofs are good, we
        // accumulate. Otherwise we drop the packet and move on.
        let peer_validation_packets: HashMap<Uuid, ValidationPacket> = peer_validation_packets
            .into_iter()
            .map(|p| (p.uuid, p))
            .collect();

        // Keep track of the ingestion packets we have seen so we can reject
        // duplicates.
        let mut processed_ingestion_packets = HashSet::new();

        // Convert ingestion packets into validation packets; if we can't
        // decrypt a packet, drop/skip the entire ingestion batch without
        // error. (Other errors are reported to the caller as normal & will
        // cause the entire aggregation batch to fail.)
        let mut ingestion_and_own_validation_packets = Vec::new();
        for ingestion_packet in ingestion_packets {
            match ingestion_packet.generate_validation_packet(servers) {
                Ok(p) => ingestion_and_own_validation_packets.push((ingestion_packet, p)),
                Err(e) => {
                    if let IntakeError::PacketDecryption(packet_uuid) = e {
                        // This ingestion batch contains a packet that can't be
                        // decrypted. Ignore the entire ingestion batch, on the
                        // assumption that our peer will do the same, since we
                        // also wouldn't have been able to produce a peer
                        // validation batch from this ingestion batch.
                        warn!(self.logger, "Skipping ingestion batch due to undecryptable packet";
                            event::PACKET_UUID => packet_uuid.to_string());
                        return Ok(false);
                    }
                    return Err(AggregationError::Intake(e));
                }
            }
        }

        // Borrowing distinct parts of a struct works, but not under closures:
        // https://github.com/rust-lang/rust/issues/53488
        // The workaround is to borrow or copy fields outside the closure.
        let logger = &self.logger;

        for (ingestion_packet, own_validation_packet) in ingestion_and_own_validation_packets {
            // Ignore duplicate packets
            if processed_ingestion_packets.contains(&ingestion_packet.uuid) {
                warn!(
                    logger, "Ignoring duplicate packet";
                    event::PACKET_UUID => ingestion_packet.uuid.to_string()
                );
                continue;
            }

            // Make sure we a peer validation packet matching the ingestion packet.
            let peer_validation_packet = match peer_validation_packets.get(&ingestion_packet.uuid) {
                Some(p) => p,
                None => {
                    warn!(
                        logger, "No peer validation packet";
                        event::PACKET_UUID => ingestion_packet.uuid.to_string()
                    );
                    invalid_uuids.push(ingestion_packet.uuid);
                    continue;
                }
            };

            processed_ingestion_packets.insert(ingestion_packet.uuid);

            let mut did_aggregate_shares = false;
            let mut last_err = None;
            for server in servers.iter_mut() {
                match server.aggregate(
                    &ingestion_packet.encrypted_payload,
                    &VerificationMessage::try_from(peer_validation_packet)
                        .map_err(AggregationError::InvalidVerification)?,
                    &VerificationMessage::try_from(&own_validation_packet)
                        .map_err(AggregationError::InvalidVerification)?,
                ) {
                    Ok(valid) => {
                        if !valid {
                            warn!(
                                logger, "Rejecting packet due to invalid proof";
                                event::PACKET_UUID => peer_validation_packet.uuid.to_string(),
                            );
                            invalid_uuids.push(peer_validation_packet.uuid);
                        }
                        self.total_individual_clients += 1;
                        self.bytes_processed += ingestion_packet.encrypted_payload.len() as u64;

                        did_aggregate_shares = true;
                        break;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        continue;
                    }
                }
            }
            if !did_aggregate_shares {
                match last_err {
                    Some(e) => return Err(AggregationError::Validation(e.into())),
                    None => {
                        return Err(AggregationError::Validation(anyhow::anyhow!(
                            "unknown validation error"
                        )))
                    }
                }
            }
        }

        if let Some(collector) = self.metrics_collector {
            collector
                .packets_per_batch
                .with_label_values(&[self.aggregation_name])
                .set(processed_ingestion_packets.len() as i64);
        }

        Ok(true)
    }
}
