use crate::{
    batch::{Batch, BatchReader, BatchWriter},
    idl::{
        IngestionDataSharePacket, IngestionHeader, InvalidPacket, Packet, SumPart,
        ValidationHeader, ValidationPacket,
    },
    metrics::AggregateMetricsCollector,
    transport::{SignableTransport, VerifiableAndDecryptableTransport, VerifiableTransport},
    BatchSigningKey, Error,
};
use anyhow::{Context, Result};
use avro_rs::Reader;
use chrono::NaiveDateTime;
use log::info;
use prio::server::{Server, VerificationMessage};
use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    io::Cursor,
};
use uuid::Uuid;

pub struct BatchAggregator<'a> {
    is_first: bool,
    aggregation_name: &'a str,
    aggregation_start: &'a NaiveDateTime,
    aggregation_end: &'a NaiveDateTime,
    own_validation_transport: &'a mut VerifiableTransport,
    peer_validation_transport: &'a mut VerifiableTransport,
    ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
    aggregation_batch: BatchWriter<'a, SumPart, InvalidPacket>,
    share_processor_signing_key: &'a BatchSigningKey,
    total_individual_clients: i64,
    metrics_collector: Option<&'a AggregateMetricsCollector>,
}

impl<'a> BatchAggregator<'a> {
    #[allow(clippy::too_many_arguments)] // Grandfathered in
    pub fn new(
        instance_name: &'a str,
        aggregation_name: &'a str,
        aggregation_start: &'a NaiveDateTime,
        aggregation_end: &'a NaiveDateTime,
        is_first: bool,
        ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
        own_validation_transport: &'a mut VerifiableTransport,
        peer_validation_transport: &'a mut VerifiableTransport,
        aggregation_transport: &'a mut SignableTransport,
    ) -> Result<BatchAggregator<'a>> {
        Ok(BatchAggregator {
            is_first,
            aggregation_name,
            aggregation_start,
            aggregation_end,
            own_validation_transport,
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
            ),
            share_processor_signing_key: &aggregation_transport.batch_signing_key,
            total_individual_clients: 0,
            metrics_collector: None,
        })
    }

    pub fn set_metrics_collector(&mut self, collector: &'a AggregateMetricsCollector) {
        self.metrics_collector = Some(collector);
    }

    /// Compute the sum part for all the provided batch IDs and write it out to
    /// the aggregation transport. The provided callback is invoked after each
    /// batch is aggregated.
    pub fn generate_sum_part<F: FnMut()>(
        &mut self,
        batch_ids: &[(Uuid, NaiveDateTime)],
        mut callback: F,
    ) -> Result<(), Error> {
        info!(
            "processing intake from {}, own validity from {}, peer validity from {} and saving sum parts to {}",
            self.ingestion_transport.transport.transport.path(),
            self.own_validation_transport.transport.path(),
            self.peer_validation_transport.transport.path(),
            self.aggregation_batch.path(),
        );
        let mut invalid_uuids = Vec::new();
        let mut included_batch_uuids = Vec::new();

        let ingestion_header = self.ingestion_header(&batch_ids[0].0, &batch_ids[0].1)?;

        // Ideally, we would use the encryption_key_id in the ingestion packet
        // to figure out which private key to use for decryption, but that field
        // is optional. Instead we try all the keys we have available until one
        // works.
        // https://github.com/abetterinternet/prio-server/issues/73
        let mut servers = self
            .ingestion_transport
            .packet_decryption_keys
            .iter()
            .map(|k| Server::new(ingestion_header.bins as usize, self.is_first, k.clone()))
            .collect::<Vec<Server>>();

        for batch_id in batch_ids {
            self.aggregate_share(&batch_id.0, &batch_id.1, &mut servers, &mut invalid_uuids)?;
            included_batch_uuids.push(batch_id.0);
            callback();
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
        );
        for server in servers.iter() {
            accumulator_server.merge_total_shares(server.total_shares());
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
        servers: &mut Vec<Server>,
        invalid_uuids: &mut Vec<Uuid>,
    ) -> Result<(), Error> {
        let mut ingestion_batch: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(self.aggregation_name, batch_id, batch_date),
                &mut *self.ingestion_transport.transport.transport,
            );
        let mut own_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(self.aggregation_name, batch_id, batch_date, self.is_first),
                &mut *self.own_validation_transport.transport,
            );
        let mut peer_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(self.aggregation_name, batch_id, batch_date, !self.is_first),
                &mut *self.peer_validation_transport.transport,
            );

        let peer_validation_header = match peer_validation_batch
            .header(&self.peer_validation_transport.batch_signing_public_keys)
        {
            Ok(header) => header,
            Err(error) => {
                if let Some(collector) = self.metrics_collector {
                    collector
                        .invalid_peer_validation_batches
                        .with_label_values(&["header"])
                        .inc();
                }
                return Err(Error::BadPeerValidationBatch(error.into()));
            }
        };

        let own_validation_header = match own_validation_batch
            .header(&self.own_validation_transport.batch_signing_public_keys)
        {
            Ok(header) => header,
            Err(error) => {
                if let Some(collector) = self.metrics_collector {
                    collector
                        .invalid_own_validation_batches
                        .with_label_values(&["header"])
                        .inc();
                }
                return Err(Error::BadOwnValidationBatch(error.into()));
            }
        };

        let ingestion_header = ingestion_batch
            .header(&self.ingestion_transport.transport.batch_signing_public_keys)
            .map_err(|e| Error::BadIngestionBatch(e.into()))?;

        // Make sure all the parameters in the headers line up
        if !peer_validation_header.check_parameters(&own_validation_header) {
            return Err(Error::BadPeerValidationBatch(
                Error::HeaderMismatch {
                    lhs: format!("{:?}", peer_validation_header),
                    rhs: format!("{:?}", own_validation_header),
                }
                .into(),
            ));
        }
        if !ingestion_header.check_parameters(&peer_validation_header) {
            return Err(Error::BadOwnValidationBatch(
                Error::HeaderMismatch {
                    lhs: format!("{:?}", ingestion_header),
                    rhs: format!("{:?}", peer_validation_header),
                }
                .into(),
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
            match peer_validation_batch.packet_file_reader(&peer_validation_header) {
                Ok(reader) => reader,
                Err(error) => {
                    if let Some(collector) = self.metrics_collector {
                        collector
                            .invalid_peer_validation_batches
                            .with_label_values(&["packet_file"])
                            .inc();
                    }
                    return Err(Error::BadPeerValidationBatch(error.into()));
                }
            };
        let peer_validation_packets: HashMap<Uuid, ValidationPacket> =
            validation_packet_map(&mut peer_validation_packet_file_reader)?;

        let mut own_validation_packet_file_reader =
            match own_validation_batch.packet_file_reader(&own_validation_header) {
                Ok(reader) => reader,
                Err(error) => {
                    if let Some(collector) = self.metrics_collector {
                        collector
                            .invalid_own_validation_batches
                            .with_label_values(&["packet_file"])
                            .inc();
                    }
                    return Err(Error::BadOwnValidationBatch(error.into()));
                }
            };

        let own_validation_packets: HashMap<Uuid, ValidationPacket> =
            validation_packet_map(&mut own_validation_packet_file_reader)?;

        // Keep track of the ingestion packets we have seen so we can reject
        // duplicates.
        let mut processed_ingestion_packets = HashSet::new();

        let mut ingestion_packet_reader = ingestion_batch
            .packet_file_reader(&ingestion_header)
            .map_err(|e| Error::BadIngestionBatch(e.into()))?;

        loop {
            let ingestion_packet =
                match IngestionDataSharePacket::read(&mut ingestion_packet_reader) {
                    Ok(p) => p,
                    Err(Error::Eof) => break,
                    Err(e) => return Err(e.into()),
                };

            // Ignore duplicate packets
            if processed_ingestion_packets.contains(&ingestion_packet.uuid) {
                info!("ignoring duplicate packet {}", ingestion_packet.uuid);
                continue;
            }

            // Make sure we have own validation and peer validation packets
            // matching the ingestion packet.
            let peer_validation_packet = get_validation_packet(
                &ingestion_packet.uuid,
                &peer_validation_packets,
                "peer",
                invalid_uuids,
            );
            let peer_validation_packet: &ValidationPacket = match peer_validation_packet {
                Some(p) => p,
                None => continue,
            };

            let own_validation_packet = get_validation_packet(
                &ingestion_packet.uuid,
                &own_validation_packets,
                "own",
                invalid_uuids,
            );
            let own_validation_packet: &ValidationPacket = match own_validation_packet {
                Some(p) => p,
                None => continue,
            };

            processed_ingestion_packets.insert(ingestion_packet.uuid);

            let mut did_aggregate_shares = false;
            let mut last_err = None;
            for server in servers.iter_mut() {
                match server.aggregate(
                    &ingestion_packet.encrypted_payload,
                    &VerificationMessage::try_from(peer_validation_packet).context(
                        "peer validation packet could not be converted to verification message",
                    )?,
                    &VerificationMessage::try_from(own_validation_packet).context(
                        "own validation packet could not be converted to verification message",
                    )?,
                ) {
                    Ok(valid) => {
                        if !valid {
                            info!(
                                "rejecting packet {} due to invalid proof",
                                peer_validation_packet.uuid
                            );
                            invalid_uuids.push(peer_validation_packet.uuid);
                        }
                        self.total_individual_clients += 1;
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
                return Ok(last_err
                    // Unwrap the optional, providing an error if it is None
                    .context("unknown validation error")?
                    // Wrap either the default error or what we got from
                    // server.aggregate
                    .context("failed to validate packets")?);
            }
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
            Err(Error::Eof) => return Ok(map),
            Err(e) => return Err(e.into()),
        }
    }
}

fn get_validation_packet<'a>(
    uuid: &Uuid,
    validation_packets: &'a HashMap<Uuid, ValidationPacket>,
    kind: &str,
    invalid_uuids: &mut Vec<Uuid>,
) -> Option<&'a ValidationPacket> {
    match validation_packets.get(uuid) {
        None => {
            info!("no {} validation packet for {}", kind, uuid);
            invalid_uuids.push(*uuid);
            None
        }
        result => result,
    }
}
