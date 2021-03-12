use crate::{
    batch::{Batch, BatchReader, BatchWriter},
    idl::{
        IngestionDataSharePacket, IngestionHeader, InvalidPacket, Packet, SumPart,
        ValidationHeader, ValidationPacket,
    },
    metrics::AggregateMetricsCollector,
    task::Watchdog,
    transport::{SignableTransport, VerifiableAndDecryptableTransport, VerifiableTransport},
    work_queue::WorkQueue,
    BatchSigningKey, Error,
};
use anyhow::{anyhow, Context, Result};
use avro_rs::Reader;
use chrono::NaiveDateTime;
use log::{debug, info};
use prio::{
    finite_field::{merge_vector, Field},
    server::{Server, VerificationMessage},
    util::vector_with_length,
};
use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    io::Cursor,
    thread::{self, JoinHandle},
};
use uuid::Uuid;

/// A batch included in an aggregation.
#[derive(Debug, Clone)]
pub struct SumInput {
    pub uuid: Uuid,
    pub time: NaiveDateTime,
}

/// The result of summing an individual batch
#[derive(Debug, Clone)]
struct SumOutput {
    individual_client_count: usize,
    sum: Vec<Field>,
    invalid_packet_uuids: Vec<Uuid>,
}

/// BatchAggregator aggregates over multiple batches of shares and produces a
/// single SumPart.
pub struct BatchAggregator<'a> {
    trace_id: &'a str,
    is_first: bool,
    aggregation_name: &'a str,
    aggregation_start: &'a NaiveDateTime,
    aggregation_end: &'a NaiveDateTime,
    own_validation_transport: &'a mut VerifiableTransport,
    peer_validation_transport: &'a mut VerifiableTransport,
    ingestion_transport: &'a mut VerifiableAndDecryptableTransport,
    aggregation_batch: BatchWriter<'a, SumPart, InvalidPacket>,
    share_processor_signing_key: &'a BatchSigningKey,
    metrics_collector: Option<&'a AggregateMetricsCollector>,
    thread_count: u32,
}

impl<'a> BatchAggregator<'a> {
    #[allow(clippy::too_many_arguments)] // Grandfathered in
    pub fn new(
        trace_id: &'a str,
        instance_name: &str,
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
            trace_id,
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
            metrics_collector: None,
            thread_count: 1,
        })
    }

    pub fn set_metrics_collector(&mut self, collector: &'a AggregateMetricsCollector) {
        self.metrics_collector = Some(collector);
    }

    /// Set the number of worker threads that will be spawned to sum batches
    pub fn set_thread_count(&mut self, count: u32) {
        self.thread_count = count;
    }

    /// Compute the sum part for all the provided batch IDs and write it out to
    /// the aggregation transport. The provided callback is invoked after each
    /// batch is aggregated.
    pub fn generate_sum_part<W: Watchdog>(
        &mut self,
        batches: Vec<SumInput>,
        watchdog: &W,
    ) -> Result<()> {
        info!(
            "trace id {} processing intake from {}, own validity from {}, peer validity from {} and saving sum parts to {}",
            self.trace_id,
            self.ingestion_transport.transport.transport.path(),
            self.own_validation_transport.transport.path(),
            self.peer_validation_transport.transport.path(),
            self.aggregation_batch.path(),
        );

        let included_batch_uuids: Vec<Uuid> = batches.iter().map(|b| b.uuid).collect();
        let ingestion_header = self.ingestion_header(&batches[0].uuid, &batches[0].time)?;
        let dimension = ingestion_header.bins as usize;

        let work_queue: WorkQueue<SumInput, SumOutput> = WorkQueue::new(batches);
        let mut thread_handles: Vec<JoinHandle<Result<()>>> = Vec::new();

        debug!("spawning {} aggregation worker threads", self.thread_count);
        for _ in 0..self.thread_count {
            // Clone various objects needed by workers so they can be safely
            // moved into thread closure
            let mut queue_clone = work_queue.clone();
            let watchdog_clone = watchdog.clone();
            let mut share_aggregator = ShareAggregator {
                aggregation_name: self.aggregation_name.to_string(),
                is_first: self.is_first,
                own_validation_transport: self.own_validation_transport.clone(),
                peer_validation_transport: self.peer_validation_transport.clone(),
                ingestion_transport: self.ingestion_transport.clone(),
                metrics_collector: self.metrics_collector.cloned(),
                trace_id: self.trace_id.to_string(),
                dimension,
            };
            thread_handles.push(thread::spawn(move || {
                while let Some(job) = queue_clone.dequeue_job() {
                    let sum_output = share_aggregator.aggregate(&job)?;

                    queue_clone.send_results(sum_output);
                    watchdog_clone.send_heartbeat();
                }

                // There are no jobs left for this worker
                Ok(())
            }));
        }

        for handle in thread_handles {
            // handle::join() returns std::thread::Result<anyhow::Result<()>.
            // The first ? unwraps the outer result from ThreadHandle::join()
            // and the second handles the result from the thread closure.
            handle
                .join()
                .map_err(|e| anyhow!("aggregation worker thread panicked: {:?}", e))??;
        }

        // Reduce over all the results produced by workers.
        let results: Vec<SumOutput> = work_queue.results()?;

        let mut accumulated_sum: Vec<Field> = vector_with_length(dimension as usize);
        let mut total_individual_clients = 0;
        let mut invalid_packet_uuids: Vec<Uuid> = Vec::new();
        for mut result in results {
            merge_vector(&mut accumulated_sum, &result.sum)?;
            total_individual_clients += result.individual_client_count;
            invalid_packet_uuids.append(&mut result.invalid_packet_uuids);
        }

        // TODO(timg) what exactly do we write out when there are no invalid
        // packets? Right now we will write an empty file.
        let invalid_packets_digest =
            self.aggregation_batch
                .packet_file_writer(|mut packet_file_writer| {
                    for invalid_uuid in &invalid_packet_uuids {
                        InvalidPacket {
                            uuid: *invalid_uuid,
                        }
                        .write(&mut packet_file_writer)?
                    }
                    Ok(())
                })?;

        // Convert Vec<Field> and usize into the types expected by the Avro
        // encoding
        let accumulated_sum: Vec<i64> = accumulated_sum
            .iter()
            .map(|f| u32::from(*f) as i64)
            .collect();

        let total_individual_clients = i64::try_from(total_individual_clients)
            .context("could not represent number of contributions as signed 64 bit integer")?;

        let sum_signature = self.aggregation_batch.put_header(
            &SumPart {
                batch_uuids: included_batch_uuids,
                name: ingestion_header.name,
                bins: ingestion_header.bins,
                epsilon: ingestion_header.epsilon,
                prime: ingestion_header.prime,
                number_of_servers: ingestion_header.number_of_servers,
                hamming_weight: ingestion_header.hamming_weight,
                sum: accumulated_sum,
                aggregation_start_time: self.aggregation_start.timestamp_millis(),
                aggregation_end_time: self.aggregation_end.timestamp_millis(),
                packet_file_digest: invalid_packets_digest.as_ref().to_vec(),
                total_individual_clients: total_individual_clients as i64,
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
}

/// ShareAggregator produces sums over a single batch of shares. The intent is
/// to make it easier to understand which objects owned by BatchAggregator are
/// being cloned and sent into worker threads.
struct ShareAggregator {
    aggregation_name: String,
    is_first: bool,
    own_validation_transport: VerifiableTransport,
    peer_validation_transport: VerifiableTransport,
    ingestion_transport: VerifiableAndDecryptableTransport,
    metrics_collector: Option<AggregateMetricsCollector>,
    trace_id: String,
    dimension: usize,
}

impl ShareAggregator {
    /// Aggregate the shares in the provided batch. Returns a SumOutput
    /// containing the result of this batch's sum.
    fn aggregate(&mut self, sum_input: &SumInput) -> Result<SumOutput> {
        let mut ingestion_batch: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(&self.aggregation_name, &sum_input.uuid, &sum_input.time),
                &mut *self.ingestion_transport.transport.transport,
            );
        let mut own_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(
                    &self.aggregation_name,
                    &sum_input.uuid,
                    &sum_input.time,
                    self.is_first,
                ),
                &mut *self.own_validation_transport.transport,
            );
        let mut peer_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(
                    &self.aggregation_name,
                    &sum_input.uuid,
                    &sum_input.time,
                    !self.is_first,
                ),
                &mut *self.peer_validation_transport.transport,
            );

        let peer_validation_header = match peer_validation_batch
            .header(&self.peer_validation_transport.batch_signing_public_keys)
        {
            Ok(header) => header,
            Err(error) => {
                if let Some(collector) = &self.metrics_collector {
                    collector
                        .invalid_peer_validation_batches
                        .with_label_values(&["header"])
                        .inc();
                }
                return Err(error);
            }
        };

        let own_validation_header = match own_validation_batch
            .header(&self.own_validation_transport.batch_signing_public_keys)
        {
            Ok(header) => header,
            Err(error) => {
                if let Some(collector) = &self.metrics_collector {
                    collector
                        .invalid_own_validation_batches
                        .with_label_values(&["header"])
                        .inc();
                }
                return Err(error);
            }
        };

        let ingestion_header = ingestion_batch
            .header(&self.ingestion_transport.transport.batch_signing_public_keys)?;

        // Make sure all the parameters in the headers line up
        if !peer_validation_header.check_parameters(&own_validation_header) {
            return Err(anyhow!(
                "trace id {} validation headers do not match. Peer: {:?}\nOwn: {:?}",
                self.trace_id,
                peer_validation_header,
                own_validation_header
            ));
        }
        if !ingestion_header.check_parameters(&peer_validation_header) {
            return Err(anyhow!(
                "trace id {} ingestion header does not match peer validation header. Ingestion: {:?}\nPeer:{:?}",
                self.trace_id,
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
            match peer_validation_batch.packet_file_reader(&peer_validation_header) {
                Ok(reader) => reader,
                Err(error) => {
                    if let Some(collector) = &self.metrics_collector {
                        collector
                            .invalid_peer_validation_batches
                            .with_label_values(&["packet_file"])
                            .inc();
                    }
                    return Err(error);
                }
            };
        let peer_validation_packets: HashMap<Uuid, ValidationPacket> =
            validation_packet_map(&mut peer_validation_packet_file_reader)?;

        let mut own_validation_packet_file_reader =
            match own_validation_batch.packet_file_reader(&own_validation_header) {
                Ok(reader) => reader,
                Err(error) => {
                    if let Some(collector) = &self.metrics_collector {
                        collector
                            .invalid_own_validation_batches
                            .with_label_values(&["packet_file"])
                            .inc();
                    }
                    return Err(error);
                }
            };

        let own_validation_packets: HashMap<Uuid, ValidationPacket> =
            validation_packet_map(&mut own_validation_packet_file_reader)?;

        // We accumulate each batch's packets into a Server, which is
        // responsible for decrypting shares and verifying proofs. Ideally, we
        // would use the encryption_key_id in the ingestion packet to figure out
        // which private key to use for decryption, but that field is
        // optional[1]. Instead we try all the keys we have available until one
        // works, meaning we have to create one Server per key.
        //
        // We have live mutable borrows of self which forbid us from immutably
        // borrowing self in the closure passed to map()[2], so we have to make
        // safe copies of these values.
        //
        // [1] https://github.com/abetterinternet/prio-server/issues/73
        // [2] https://github.com/rust-lang/rust/issues/53488
        let dimension = self.dimension;
        let is_first = self.is_first;
        let mut servers: Vec<Server> = self
            .ingestion_transport
            .packet_decryption_keys
            .iter()
            .map(|k| Server::new(dimension, is_first, k.clone()))
            .collect();
        // Keep track of the ingestion packets we have seen so we can reject
        // duplicates.
        let mut processed_ingestion_packets = HashSet::new();
        let mut ingestion_packet_reader = ingestion_batch.packet_file_reader(&ingestion_header)?;
        let mut invalid_packet_uuids: Vec<Uuid> = Vec::new();

        loop {
            let ingestion_packet =
                match IngestionDataSharePacket::read(&mut ingestion_packet_reader) {
                    Ok(p) => p,
                    Err(Error::EofError) => break,
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
                &mut invalid_packet_uuids,
            );
            let peer_validation_packet: &ValidationPacket = match peer_validation_packet {
                Some(p) => p,
                None => continue,
            };

            let own_validation_packet = get_validation_packet(
                &ingestion_packet.uuid,
                &own_validation_packets,
                "own",
                &mut invalid_packet_uuids,
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
                    &VerificationMessage::try_from(peer_validation_packet)?,
                    &VerificationMessage::try_from(own_validation_packet)?,
                ) {
                    Ok(valid) => {
                        if !valid {
                            info!(
                                "trace id {} rejecting packet {} due to invalid proof",
                                self.trace_id, peer_validation_packet.uuid
                            );
                            invalid_packet_uuids.push(peer_validation_packet.uuid);
                        }
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

        let mut sum: Vec<Field> = vector_with_length(self.dimension);
        for server in &servers {
            merge_vector(&mut sum, server.total_shares())
                .context("unable to sum across per-packet-decryption-key servers")?;
        }
        Ok(SumOutput {
            individual_client_count: processed_ingestion_packets.len(),
            sum,
            invalid_packet_uuids,
        })
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
