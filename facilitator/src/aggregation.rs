use crate::{
    batch::{Batch, BatchReader, BatchWriter},
    idl::{
        IngestionDataSharePacket, IngestionHeader, InvalidPacket, Packet, SumPart,
        ValidationHeader, ValidationPacket,
    },
    logging::event,
    metrics::AggregateMetricsCollector,
    transport::{SignableTransport, VerifiableAndDecryptableTransport, VerifiableTransport},
    BatchSigningKey, Error,
};
use anyhow::{anyhow, Context, Result};
use avro_rs::Reader;
use chrono::NaiveDateTime;
use prio::{
    field::FieldPriov2,
    server::{Server, VerificationMessage},
};
use slog::{info, o, warn, Logger};
use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    io::Cursor,
};
use uuid::Uuid;

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
    #[allow(clippy::too_many_arguments)] // Grandfathered in
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
    ) -> Result<Self> {
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
                &mut *aggregation_transport.transport,
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
        batch_ids: &[(Uuid, NaiveDateTime)],
        mut callback: F,
    ) -> Result<()>
    where
        F: FnMut(&Logger),
    {
        info!(self.logger, "processing aggregation task");
        let mut invalid_uuids = Vec::new();
        let mut included_batch_uuids = Vec::new();

        let ingestion_header = self.ingestion_header(&batch_ids[0].0, &batch_ids[0].1)?;

        // Ideally, we would use the encryption_key_id in the ingestion packet
        // to figure out which private key to use for decryption, but that field
        // is optional. Instead we try all the keys we have available until one
        // works.
        // https://github.com/abetterinternet/prio-server/issues/73
        let mut servers: Vec<Server<FieldPriov2>> = self
            .ingestion_transport
            .packet_decryption_keys
            .iter()
            .map(|k| Server::new(ingestion_header.bins as usize, self.is_first, k.clone()))
            .collect::<Result<_, _>>()?;

        for batch_id in batch_ids {
            self.aggregate_share(&batch_id.0, &batch_id.1, &mut servers, &mut invalid_uuids)?;
            included_batch_uuids.push(batch_id.0);
            callback(&self.logger);
        }

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

        // TODO(timg) what exactly do we write out when there are no invalid
        // packets? Right now we will write an empty file.
        let invalid_packets_digest =
            self.aggregation_batch
                .packet_file_writer(|mut packet_file_writer| {
                    for invalid_uuid in invalid_uuids {
                        InvalidPacket { uuid: invalid_uuid }.write(&mut packet_file_writer)?
                    }
                    Ok(())
                })?;

        // We have one Server for each packet decryption key, and each of those
        // instances could contain some accumulated shares, depending on which
        // key was used to encrypt an individual packet. We make a new Server
        // instance into which we will aggregate them all together. It doesn't
        // matter which private key we use here as we're not decrypting any
        // packets with this Server instance, just accumulating data vectors.
        let mut accumulator_server = Server::new(
            ingestion_header.bins as usize,
            self.is_first,
            self.ingestion_transport.packet_decryption_keys[0].clone(),
        )
        .context("failed to construct Prio server")?;
        for server in servers.iter() {
            accumulator_server
                .merge_total_shares(server.total_shares())
                .context("failed to accumulate shares")?;
        }

        let sum = accumulator_server
            .total_shares()
            .iter()
            .map(|f| u32::from(*f) as i64)
            .collect();

        let sum_signature = self.aggregation_batch.put_header(
            &SumPart {
                batch_uuids: included_batch_uuids,
                name: ingestion_header.name,
                bins: ingestion_header.bins,
                epsilon: ingestion_header.epsilon,
                prime: ingestion_header.prime,
                number_of_servers: ingestion_header.number_of_servers,
                hamming_weight: ingestion_header.hamming_weight,
                sum,
                aggregation_start_time: self.aggregation_start.timestamp_millis(),
                aggregation_end_time: self.aggregation_end.timestamp_millis(),
                packet_file_digest: invalid_packets_digest.as_ref().to_vec(),
                total_individual_clients: self.total_individual_clients,
            },
            &self.share_processor_signing_key.key,
        )?;

        self.aggregation_batch
            .put_signature(&sum_signature, &self.share_processor_signing_key.identifier)
    }

    /// Fetch the ingestion header from one of the batches so various parameters
    /// may be read from it.
    fn ingestion_header(
        &mut self,
        batch_id: &Uuid,
        batch_date: &NaiveDateTime,
    ) -> Result<IngestionHeader> {
        let mut ingestion_batch: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(self.aggregation_name, batch_id, batch_date),
                &mut *self.ingestion_transport.transport.transport,
                self.permit_malformed_batch,
                self.trace_id,
                &self.logger,
            );
        let ingestion_header = ingestion_batch
            .header(&self.ingestion_transport.transport.batch_signing_public_keys)?;
        Ok(ingestion_header)
    }

    /// Aggregate the batch for the provided batch_id into the provided server.
    /// The UUIDs of packets for which aggregation fails are recorded in the
    /// provided invalid_uuids vector.
    fn aggregate_share(
        &mut self,
        batch_id: &Uuid,
        batch_date: &NaiveDateTime,
        servers: &mut Vec<Server<FieldPriov2>>,
        invalid_uuids: &mut Vec<Uuid>,
    ) -> Result<()> {
        let mut ingestion_batch: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(self.aggregation_name, batch_id, batch_date),
                &mut *self.ingestion_transport.transport.transport,
                self.permit_malformed_batch,
                self.trace_id,
                &self.logger,
            );
        let mut peer_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(self.aggregation_name, batch_id, batch_date, !self.is_first),
                &mut *self.peer_validation_transport.transport,
                self.permit_malformed_batch,
                self.trace_id,
                &self.logger,
            );

        if let Some(collector) = self.metrics_collector {
            peer_validation_batch
                .set_metrics_collector(&collector.peer_validation_batches_reader_metrics);
        }

        let peer_validation_header = peer_validation_batch
            .header(&self.peer_validation_transport.batch_signing_public_keys)?;

        let ingestion_header = ingestion_batch
            .header(&self.ingestion_transport.transport.batch_signing_public_keys)?;

        // Make sure all the parameters in the headers line up
        if !ingestion_header.check_parameters(&peer_validation_header) {
            return Err(anyhow!(
                "ingestion header does not match peer validation header. Ingestion: {:?}\nPeer:{:?}",
                ingestion_header,
                peer_validation_header
            ));
        }

        // We can't be sure that the peer validation, own validation and
        // ingestion batches contain all the same packets or that they are in
        // the same order. There could also be duplicate packets. Compared to
        // the ingestion packets, the validation packets are small (they only
        // contain a UUID and three i64s), so we construct a hashmap of the
        // validation batches, mapping packet UUID to a ValidationPacket, then
        // iterate over the ingestion packets. For each ingestion packet, if we
        // have the corresponding validation packets and the proofs are good, we
        // accumulate. Otherwise we drop the packet and move on.
        let mut peer_validation_packet_file_reader =
            peer_validation_batch.packet_file_reader(&peer_validation_header)?;
        let peer_validation_packets: HashMap<Uuid, ValidationPacket> =
            validation_packet_map(&mut peer_validation_packet_file_reader)?;

        // Keep track of the ingestion packets we have seen so we can reject
        // duplicates.
        let mut processed_ingestion_packets = HashSet::new();
        let mut ingestion_packet_reader = ingestion_batch.packet_file_reader(&ingestion_header)?;

        // Convert ingestion packets into validation packets; if we can't
        // decrypt a packet, drop/skip the entire ingestion batch without
        // error. (Other errors are reported to the caller as normal & will
        // cause the entire aggregation batch to fail.)
        let mut ingestion_and_own_validation_packets = Vec::new();
        loop {
            let ingestion_packet =
                match IngestionDataSharePacket::read(&mut ingestion_packet_reader) {
                    Ok(p) => p,
                    Err(Error::EofError) => break,
                    Err(e) => return Err(e.into()),
                };
            match ingestion_packet.generate_validation_packet(servers) {
                Ok(p) => ingestion_and_own_validation_packets.push((ingestion_packet, p)),
                Err(e) => {
                    if let Some(Error::PacketDecryptionError(packet_uuid)) = e.downcast_ref() {
                        // This ingestion batch contains a packet that can't be
                        // decrypted. Ignore the entire ingestion batch, on the
                        // assumption that our peer will do the same, since we
                        // also wouldn't have been able to produce a peer
                        // validation batch from this ingestion batch.
                        warn!(self.logger, "Skipping ingestion batch due to undecryptable packet";
                            event::PACKET_UUID => packet_uuid.to_string());
                        return Ok(());
                    }
                    return Err(e);
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
                    &VerificationMessage::try_from(peer_validation_packet)?,
                    &VerificationMessage::try_from(&own_validation_packet)?,
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
                        last_err = Some(Err(e));
                        continue;
                    }
                }
            }
            if !did_aggregate_shares {
                return last_err
                    // Unwrap the optional, providing an error if it is None
                    .context("unknown validation error")?
                    // Wrap either the default error or what we got from
                    // server.aggregate
                    .context(format!(
                        "trace id {} failed to validate packets",
                        self.trace_id
                    ));
            }
        }

        if let Some(collector) = self.metrics_collector {
            collector
                .packets_per_batch
                .with_label_values(&[self.aggregation_name])
                .set(processed_ingestion_packets.len() as i64);
        }

        Ok(())
    }
}

fn validation_packet_map(
    reader: &mut Reader<Cursor<Vec<u8>>>,
) -> Result<HashMap<Uuid, ValidationPacket>> {
    let mut map = HashMap::new();
    loop {
        match ValidationPacket::read(reader) {
            Ok(p) => {
                map.insert(p.uuid, p);
            }
            Err(Error::EofError) => return Ok(map),
            Err(e) => return Err(e.into()),
        }
    }
}
