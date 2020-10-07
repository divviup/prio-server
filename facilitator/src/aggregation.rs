use crate::{
    batch::{Batch, BatchReader, BatchWriter},
    idl::{
        IngestionDataSharePacket, IngestionHeader, InvalidPacket, Packet, SumPart,
        ValidationHeader, ValidationPacket,
    },
    transport::Transport,
    Error,
};
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDateTime;
use prio::{encrypt::PrivateKey, server::VerificationMessage};
use ring::signature::{EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
use std::convert::TryFrom;
use uuid::Uuid;

pub struct BatchAggregator<'a> {
    is_first: bool,
    aggregation_name: &'a str,
    aggregation_start: &'a NaiveDateTime,
    aggregation_end: &'a NaiveDateTime,
    own_validation_transport: &'a mut dyn Transport,
    peer_validation_transport: &'a mut dyn Transport,
    ingestion_transport: &'a mut dyn Transport,
    aggregation_batch: BatchWriter<'a, SumPart, InvalidPacket>,
    ingestor_key: &'a UnparsedPublicKey<Vec<u8>>,
    share_processor_signing_key: &'a EcdsaKeyPair,
    peer_share_processor_key: &'a UnparsedPublicKey<Vec<u8>>,
    share_processor_ecies_key: &'a PrivateKey,
}

impl<'a> BatchAggregator<'a> {
    pub fn new(
        aggregation_name: &'a str,
        aggregation_start: &'a NaiveDateTime,
        aggregation_end: &'a NaiveDateTime,
        is_first: bool,
        ingestion_transport: &'a mut dyn Transport,
        own_validation_transport: &'a mut dyn Transport,
        peer_validation_transport: &'a mut dyn Transport,
        aggregation_transport: &'a mut dyn Transport,
        ingestor_key: &'a UnparsedPublicKey<Vec<u8>>,
        share_processor_signing_key: &'a EcdsaKeyPair,
        peer_share_processor_key: &'a UnparsedPublicKey<Vec<u8>>,
        share_processor_ecies_key: &'a PrivateKey,
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
                    aggregation_name,
                    aggregation_start,
                    aggregation_end,
                    is_first,
                ),
                aggregation_transport,
            ),
            ingestor_key,
            share_processor_signing_key,
            peer_share_processor_key,
            share_processor_ecies_key,
        })
    }

    /// Compute the sum part for all the provided batch IDs and write it out to
    /// the aggregation transport.
    pub fn generate_sum_part(&mut self, batch_ids: &[(Uuid, NaiveDateTime)]) -> Result<()> {
        let share_processor_public_key = UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_FIXED,
            Vec::from(self.share_processor_signing_key.public_key().as_ref()),
        );
        let mut invalid_uuids = Vec::new();

        let ingestion_header = self.ingestion_header(&batch_ids[0].0, &batch_ids[0].1)?;
        let mut server = prio::server::Server::new(
            ingestion_header.bins as usize,
            self.is_first,
            self.share_processor_ecies_key.clone(),
        );

        for batch_id in batch_ids {
            self.aggregate_share(
                &batch_id.0,
                &batch_id.1,
                &share_processor_public_key,
                &mut server,
                &mut invalid_uuids,
            )?;
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

        let sum = server
            .total_shares()
            .iter()
            .map(|f| u32::from(*f) as i64)
            .collect();

        let sum_signature = self.aggregation_batch.put_header(
            &SumPart {
                batch_uuids: batch_ids.iter().map(|pair| pair.0).collect(),
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
            },
            &self.share_processor_signing_key,
        )?;

        self.aggregation_batch.put_signature(&sum_signature)
    }

    /// Fetch the ingestion header from one of the batches so various parameters
    /// may be read from it.
    fn ingestion_header(
        &mut self,
        batch_id: &Uuid,
        batch_date: &NaiveDateTime,
    ) -> Result<IngestionHeader> {
        let ingestion_batch: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(self.aggregation_name, batch_id, batch_date),
                self.ingestion_transport,
            );
        let ingestion_header = ingestion_batch.header(&self.ingestor_key)?;
        Ok(ingestion_header)
    }

    /// Aggregate the batch for the provided batch_id into the provided server.
    /// The UUIDs of packets for which aggregation fails are recorded in the
    /// provided invalid_uuids vector.
    fn aggregate_share(
        &mut self,
        batch_id: &Uuid,
        batch_date: &NaiveDateTime,
        share_processor_public_key: &UnparsedPublicKey<Vec<u8>>,
        server: &mut prio::server::Server,
        invalid_uuids: &mut Vec<Uuid>,
    ) -> Result<()> {
        let ingestion_batch: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new(
                Batch::new_ingestion(self.aggregation_name, batch_id, batch_date),
                self.ingestion_transport,
            );
        let own_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(self.aggregation_name, batch_id, batch_date, self.is_first),
                self.own_validation_transport,
            );
        let peer_validation_batch: BatchReader<'_, ValidationHeader, ValidationPacket> =
            BatchReader::new(
                Batch::new_validation(self.aggregation_name, batch_id, batch_date, !self.is_first),
                self.peer_validation_transport,
            );
        let peer_validation_header =
            peer_validation_batch.header(&self.peer_share_processor_key)?;
        let own_validation_header = own_validation_batch.header(share_processor_public_key)?;
        let ingestion_header = ingestion_batch.header(&self.ingestor_key)?;

        // Make sure all the parameters in the headers line up
        if !peer_validation_header.check_parameters(&own_validation_header) {
            return Err(anyhow!(
                "validation headers do not match. Peer: {:?}\nOwn: {:?}",
                peer_validation_header,
                own_validation_header
            ));
        }
        if !ingestion_header.check_parameters(&peer_validation_header) {
            return Err(anyhow!(
                "ingestion header does not match peer validation header. Ingestion: {:?}\nPeer:{:?}",
                ingestion_header,
                peer_validation_header
            ));
        }

        let mut peer_validation_packet_reader =
            peer_validation_batch.packet_file_reader(&peer_validation_header)?;
        let mut own_validation_packet_reader =
            own_validation_batch.packet_file_reader(&own_validation_header)?;
        let mut ingestion_packet_reader = ingestion_batch.packet_file_reader(&ingestion_header)?;

        loop {
            let peer_validation_packet =
                match ValidationPacket::read(&mut peer_validation_packet_reader) {
                    Ok(p) => Some(p),
                    Err(Error::EofError) => None,
                    Err(e) => return Err(e.into()),
                };
            let own_validation_packet =
                match ValidationPacket::read(&mut own_validation_packet_reader) {
                    Ok(p) => Some(p),
                    Err(Error::EofError) => None,
                    Err(e) => return Err(e.into()),
                };
            let ingestion_packet =
                match IngestionDataSharePacket::read(&mut ingestion_packet_reader) {
                    Ok(p) => Some(p),
                    Err(Error::EofError) => None,
                    Err(e) => return Err(e.into()),
                };

            // All three packet files should contain the same number of packets,
            // so if any of the readers hit EOF before the others, something is
            // fishy.
            let (peer_validation_packet, own_validation_packet, ingestion_packet) = match (
                &peer_validation_packet,
                &own_validation_packet,
                &ingestion_packet,
            ) {
                (Some(a), Some(b), Some(c)) => (a, b, c),
                (None, None, None) => break,
                (_, _, _) => {
                    return Err(anyhow!(
                        "unexpected early EOF when checking peer validations"
                    ));
                }
            };

            // TODO(timg) we need to make sure we are evaluating a valid triple
            // of (peer validation, own validation, ingestion), i.e., they must
            // have the same UUID. Can we assume that the peer validations will
            // be in the same order as ours, or do we need to do an O(n) search
            // of the peer validation packets to find the right UUID? Further,
            // if aggregation fails, then we record the invalid UUID and send
            // that along to the aggregator. But if some UUIDs are missing from
            // the peer validation packet file (the EOF case handled above),
            // should that UUID be marked as invalid or do we abort handling of
            // the whole batch?
            // For now I am assuming that all batches maintain the same order
            // and that they are required to contain the same set of UUIDs.
            if peer_validation_packet.uuid != own_validation_packet.uuid
                || peer_validation_packet.uuid != ingestion_packet.uuid
                || own_validation_packet.uuid != ingestion_packet.uuid
            {
                return Err(anyhow!(
                    "mismatch between peer validation, own validation and ingestion packet UUIDs: {} {} {}",
                    peer_validation_packet.uuid,
                    own_validation_packet.uuid,
                    ingestion_packet.uuid));
            }

            if !server
                .aggregate(
                    &ingestion_packet.encrypted_payload,
                    &VerificationMessage::try_from(peer_validation_packet)?,
                    &VerificationMessage::try_from(own_validation_packet)?,
                )
                .context("failed to validate packets")?
            {
                invalid_uuids.push(peer_validation_packet.uuid);
            }
        }

        Ok(())
    }
}
