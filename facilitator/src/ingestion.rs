use crate::{
    idl::{
        ingestion_data_share_packet_schema, validation_packet_schema, IngestionDataSharePacket,
        IngestionHeader, ValidationHeader, ValidationPacket,
    },
    transport::Transport,
    Error,
};
use avro_rs::{Reader, Writer};
use libprio_rs::{encrypt::PrivateKey, finite_field::Field, server::Server};
use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct Batch {
    header_path: PathBuf,
    packet_file_path: PathBuf,
    signature_path: PathBuf,
}

impl Batch {
    pub fn new_ingestion(aggregation_name: String, uuid: Uuid, date: String) -> Batch {
        Batch::new(aggregation_name, uuid, date, "batch".to_owned())
    }

    pub fn new_validation(
        aggregation_name: String,
        uuid: Uuid,
        date: String,
        is_first: bool,
    ) -> Batch {
        Batch::new(
            aggregation_name,
            uuid,
            date,
            format!("validity_{}", if is_first { 0 } else { 1 }),
        )
    }

    fn new(aggregation_name: String, uuid: Uuid, date: String, filename: String) -> Batch {
        let batch_path = PathBuf::new()
            .join(aggregation_name)
            .join(date)
            .join(uuid.to_hyphenated().to_string());

        Batch {
            header_path: batch_path.with_extension(filename.clone()),
            packet_file_path: batch_path.with_extension(format!("{}.avro", filename)),
            signature_path: batch_path.with_extension(format!("{}.sig", filename)),
        }
    }

    pub fn header_key(&self) -> &Path {
        self.header_path.as_path()
    }

    pub fn packet_file_key(&self) -> &Path {
        self.packet_file_path.as_path()
    }

    pub fn signature_path(&self) -> &Path {
        self.signature_path.as_path()
    }
}

pub struct BatchIngestor<'a> {
    ingestion_transport: &'a mut dyn Transport,
    validation_transport: &'a mut dyn Transport,
    ingestion_batch: Batch,
    validation_batch: Batch,
    is_first: bool,
    key: PrivateKey,
}

impl<'a> BatchIngestor<'a> {
    pub fn new(
        aggregation_name: String,
        uuid: Uuid,
        date: String,
        ingestion_transport: &'a mut dyn Transport,
        validation_transport: &'a mut dyn Transport,
        is_first: bool,
        key: PrivateKey,
    ) -> BatchIngestor<'a> {
        BatchIngestor {
            ingestion_transport: ingestion_transport,
            validation_transport: validation_transport,
            ingestion_batch: Batch::new_ingestion(aggregation_name.clone(), uuid, date.clone()),
            validation_batch: Batch::new_validation(aggregation_name, uuid, date, is_first),
            key: key,
            is_first: is_first,
        }
    }

    pub fn generate_validation_share(&mut self) -> Result<(), Error> {
        // TODO: fetch signature structure

        // Fetch ingestion header
        let ingestion_header_reader = self
            .ingestion_transport
            .get(self.ingestion_batch.header_key())?;
        let ingestion_header = IngestionHeader::read(ingestion_header_reader)?;

        // TODO: validate header signature
        if ingestion_header.bins <= 0 {
            return Err(Error::MalformedHeaderError(format!(
                "invalid bins/dimension value {}",
                ingestion_header.bins
            )));
        }
        let mut server = Server::new(
            ingestion_header.bins as usize,
            self.is_first,
            self.key.clone(),
        );

        // Fetch ingestion packet file
        let ingestion_packet_schema = ingestion_data_share_packet_schema();
        let ingestion_packet_file_reader = self
            .ingestion_transport
            .get(self.ingestion_batch.packet_file_key())?;
        let mut ingestion_packet_reader =
            Reader::with_schema(&ingestion_packet_schema, ingestion_packet_file_reader).map_err(
                |e| {
                    Error::AvroError(
                        "failed to create Avro reader for data share packets".to_owned(),
                        e,
                    )
                },
            )?;

        // TODO: validate packet file signature

        // Iterate over ingestion packets, compute validation share, and write
        // them to validation datastore.
        let validation_packet_schema = validation_packet_schema();
        let mut validation_packet_file_writer = self
            .validation_transport
            .put(self.validation_batch.packet_file_key())?;
        let mut validation_packet_writer = Writer::new(
            &validation_packet_schema,
            &mut validation_packet_file_writer,
        );

        loop {
            let packet = match IngestionDataSharePacket::read(&mut ingestion_packet_reader) {
                Ok(p) => p,
                Err(Error::EofError) => break,
                Err(e) => return Err(e),
            };

            let r_pit = match u32::try_from(packet.r_pit) {
                Ok(v) => v,
                Err(s) => {
                    return Err(Error::MalformedDataPacketError(format!(
                        "illegal r_pit value {} ({})",
                        packet.r_pit, s
                    )))
                }
            };

            let validation_message = match server
                .generate_verification_message(Field::from(r_pit), &packet.encrypted_payload)
            {
                Some(m) => m,
                None => {
                    return Err(Error::LibPrioError(
                        "failed to construct validation message".to_owned(),
                        None,
                    ))
                }
            };

            ValidationPacket {
                uuid: packet.uuid,
                f_r: u32::from(validation_message.f_r) as i64,
                g_r: u32::from(validation_message.g_r) as i64,
                h_r: u32::from(validation_message.h_r) as i64,
            }
            .write(&mut validation_packet_writer)?;
        }

        // Construct validation header and write it out
        let mut validation_header_writer = self
            .validation_transport
            .put(self.validation_batch.header_key())?;
        ValidationHeader {
            batch_uuid: ingestion_header.batch_uuid,
            name: ingestion_header.name,
            bins: ingestion_header.bins,
            epsilon: ingestion_header.epsilon,
            prime: ingestion_header.prime,
            number_of_servers: ingestion_header.number_of_servers,
            hamming_weight: ingestion_header.hamming_weight,
        }
        .write(&mut validation_header_writer)?;

        // TODO sign validation header and packet file and write it out

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        default_ingestor_private_key, sample::generate_ingestion_sample, transport::FileTransport,
        DEFAULT_FACILITATOR_PRIVATE_KEY, DEFAULT_PHA_PRIVATE_KEY,
    };

    #[test]
    fn share_validator() {
        let pha_tempdir = tempfile::TempDir::new().unwrap();
        let facilitator_tempdir = tempfile::TempDir::new().unwrap();

        let aggregation_name = "fake-aggregation-1".to_owned();
        let date = "fake-date".to_owned();
        let batch_uuid = Uuid::new_v4();
        let mut pha_ingest_transport = FileTransport::new(pha_tempdir.path().to_path_buf());
        let mut facilitator_ingest_transport =
            FileTransport::new(facilitator_tempdir.path().to_path_buf());
        let mut pha_validate_transport = FileTransport::new(pha_tempdir.path().to_path_buf());
        let mut facilitator_validate_transport =
            FileTransport::new(facilitator_tempdir.path().to_path_buf());
        let pha_key = PrivateKey::from_base64(DEFAULT_PHA_PRIVATE_KEY).unwrap();
        let facilitator_key = PrivateKey::from_base64(DEFAULT_FACILITATOR_PRIVATE_KEY).unwrap();

        let res = generate_ingestion_sample(
            &mut pha_ingest_transport,
            &mut facilitator_ingest_transport,
            batch_uuid,
            aggregation_name.clone(),
            date.clone(),
            &pha_key,
            &facilitator_key,
            &default_ingestor_private_key(),
            10,
            10,
            0.11,
            100,
            100,
        );
        assert!(res.is_ok(), "failed to generate sample: {:?}", res.err());

        let mut pha_ingestor = BatchIngestor::new(
            aggregation_name.clone(),
            batch_uuid,
            date.clone(),
            &mut pha_ingest_transport,
            &mut pha_validate_transport,
            true,
            pha_key,
        );

        let res = pha_ingestor.generate_validation_share();
        assert!(
            res.is_ok(),
            "PHA failed to generate validation: {:?}",
            res.err()
        );

        let mut facilitator_ingestor = BatchIngestor::new(
            aggregation_name,
            batch_uuid,
            date,
            &mut facilitator_ingest_transport,
            &mut facilitator_validate_transport,
            false,
            facilitator_key,
        );

        let res = facilitator_ingestor.generate_validation_share();
        assert!(
            res.is_ok(),
            "facilitator failed to generate validation: {:?}",
            res.err()
        );
    }
}
