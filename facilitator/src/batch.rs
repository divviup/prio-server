use crate::{
    idl::{Header, Packet},
    transport::Transport,
    DigestWriter, SidecarWriter, DATE_FORMAT,
};
use anyhow::{anyhow, Context, Result};
use avro_rs::{Reader, Schema, Writer};
use chrono::NaiveDateTime;
use ring::{
    digest::Digest,
    rand::SystemRandom,
    signature::{EcdsaKeyPair, Signature, UnparsedPublicKey},
};
use std::io::{Cursor, Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Manages the paths to the different files in a batch
pub struct Batch {
    header_path: PathBuf,
    signature_path: PathBuf,
    packet_file_path: PathBuf,
}

impl Batch {
    /// Creates a Batch representing an ingestion batch
    fn new_ingestion(aggregation_name: &str, batch_id: &Uuid, date: &NaiveDateTime) -> Batch {
        Batch::new(aggregation_name, batch_id, date, "batch")
    }

    /// Creates a Batch representing a validation batch
    fn new_validation(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        is_first: bool,
    ) -> Batch {
        Batch::new(
            aggregation_name,
            batch_id,
            date,
            &format!("validity_{}", if is_first { 0 } else { 1 }),
        )
    }

    // Creates a batch representing a sum part batch
    fn new_sum(
        aggregation_name: &str,
        aggregation_start: &NaiveDateTime,
        aggregation_end: &NaiveDateTime,
        is_first: bool,
    ) -> Batch {
        let batch_path = PathBuf::new().join(aggregation_name).join(format!(
            "{}-{}",
            aggregation_start.format(DATE_FORMAT),
            aggregation_end.format(DATE_FORMAT)
        ));
        let filename = format!("sum_{}", if is_first { 0 } else { 1 });

        Batch {
            header_path: batch_path.with_extension(&filename),
            signature_path: batch_path.with_extension(format!("{}.sig", &filename)),
            packet_file_path: batch_path.with_extension(format!(
                "invalid_uuid_{}.avro",
                if is_first { 0 } else { 1 }
            )),
        }
    }

    fn new(aggregation_name: &str, batch_id: &Uuid, date: &NaiveDateTime, filename: &str) -> Batch {
        let batch_path = PathBuf::new()
            .join(aggregation_name)
            .join(date.format(DATE_FORMAT).to_string())
            .join(batch_id.to_hyphenated().to_string());

        Batch {
            header_path: batch_path.with_extension(filename),
            signature_path: batch_path.with_extension(format!("{}.sig", filename)),
            packet_file_path: batch_path.with_extension(format!("{}.avro", filename)),
        }
    }

    fn header_key(&self) -> &Path {
        self.header_path.as_path()
    }

    fn signature_key(&self) -> &Path {
        self.signature_path.as_path()
    }

    fn packet_file_key(&self) -> &Path {
        self.packet_file_path.as_path()
    }
}

pub trait BatchIO<'a, H, P>: Sized {
    fn new_ingestion(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        transport: &'a mut dyn Transport,
    ) -> Result<Self> {
        Self::new(
            Batch::new_ingestion(aggregation_name, batch_id, date),
            transport,
        )
    }

    fn new_validation(
        aggregation_name: &str,
        batch_id: &Uuid,
        date: &NaiveDateTime,
        is_first: bool,
        transport: &'a mut dyn Transport,
    ) -> Result<Self> {
        Self::new(
            Batch::new_validation(aggregation_name, batch_id, date, is_first),
            transport,
        )
    }

    fn new_sum(
        aggregation_name: &str,
        aggregation_start: &NaiveDateTime,
        aggregation_end: &NaiveDateTime,
        is_first: bool,
        transport: &'a mut dyn Transport,
    ) -> Result<Self> {
        Self::new(
            Batch::new_sum(
                aggregation_name,
                aggregation_start,
                aggregation_end,
                is_first,
            ),
            transport,
        )
    }

    fn new(batch: Batch, transport: &'a mut dyn Transport) -> Result<Self>;
}

/// Allows reading files, including signature validation, from an ingestion or
/// validation batch containing a header, a packet file and a signature.
pub struct BatchReader<'a, H, P> {
    batch: Batch,
    transport: &'a mut dyn Transport,
    packet_schema: Schema,
    // These next two fields are not real and are used because not using H and P
    // in the struct definition is an error.
    phantom_header: PhantomData<&'a H>,
    phantom_packet: PhantomData<&'a P>,
}

impl<'a, H: Header, P: Packet> BatchIO<'a, H, P> for BatchReader<'a, H, P> {
    fn new(batch: Batch, transport: &'a mut dyn Transport) -> Result<Self> {
        Ok(BatchReader {
            batch: batch,
            transport: transport,
            packet_schema: P::schema(),
            phantom_header: PhantomData,
            phantom_packet: PhantomData,
        })
    }
}

impl<'a, H: Header, P: Packet> BatchReader<'a, H, P> {
    /// Return the parsed header from this batch, but only if its signature is
    /// valid.
    pub fn header(&self, key: &UnparsedPublicKey<Vec<u8>>) -> Result<H> {
        let mut signature = Vec::new();
        self.transport
            .get(self.batch.signature_key())?
            .read_to_end(&mut signature)
            .context("failed to read signature")?;

        let mut header_buf = Vec::new();
        self.transport
            .get(self.batch.header_key())?
            .read_to_end(&mut header_buf)
            .context("failed to read header from transport")?;

        key.verify(&header_buf, &signature)
            .context("invalid signature on header")?;

        Ok(H::read(Cursor::new(header_buf))?)
    }

    /// Return an avro_rs::Reader that yields the packets in the packet file,
    /// but only if the whole file's digest matches the packet_file_digest field
    /// in the provided header. The header is assumed to be trusted.
    pub fn packet_file_reader(&self, header: &H) -> Result<Reader<Cursor<Vec<u8>>>> {
        // Fetch packet file to validate its digest. It could be quite large so
        // so our intuition would be to stream the packets from the transport
        // and into a hasher and into the validation step, so that we wouldn't
        // need the whole file in memory at once. We can't do this because:
        //   (1) we don't want to do anything with any of the data in the packet
        //       file until we've verified integrity+authenticity
        //   (2) we need to copy the entire file into storage we control before
        //       validating its digest to avoid TOCTOU vulnerabilities.
        // We are assured by our friends writing ingestion servers that batches
        // will be no more than 300-400 MB, which fits quite reasonably into the
        // memory of anything we're going to run the facilitator on, so we load
        // the entire packet file into memory ...
        let mut packet_file_reader = self.transport.get(self.batch.packet_file_key())?;
        let entire_packet_file = Vec::new();
        let digest_writer = DigestWriter::new();
        let mut sidecar_writer = SidecarWriter::new(entire_packet_file, digest_writer);

        std::io::copy(&mut packet_file_reader, &mut sidecar_writer)
            .context("failed to load packet file")?;

        // ... then verify the digest over it ...
        if header.packet_file_digest().as_slice() != sidecar_writer.sidecar.finish().as_ref() {
            return Err(anyhow!("packet file digest does not match header"));
        }

        // ... then return a packet reader.
        Reader::with_schema(&self.packet_schema, Cursor::new(sidecar_writer.writer))
            .context("failed to create Avro reader for packets")
    }
}

/// Allows writing files, including signature file construction, from an
/// ingestion or validation batch containing a header, a packet file and a
/// signature.
pub struct BatchWriter<'a, H, P> {
    batch: Batch,
    transport: &'a mut dyn Transport,
    packet_schema: Schema,
    phantom_header: PhantomData<&'a H>,
    phantom_packet: PhantomData<&'a P>,
}

impl<'a, H: Header, P: Packet> BatchIO<'a, H, P> for BatchWriter<'a, H, P> {
    fn new(batch: Batch, transport: &'a mut dyn Transport) -> Result<Self> {
        Ok(BatchWriter {
            batch: batch,
            transport: transport,
            packet_schema: P::schema(),
            phantom_header: PhantomData,
            phantom_packet: PhantomData,
        })
    }
}

impl<'a, H: Header, P: Packet> BatchWriter<'a, H, P> {
    /// Encode the provided header into Avro, sign that representation with the
    /// provided key and write the header into the batch. Returns the signature
    /// on success.
    pub fn put_header(&mut self, header: &H, key: &EcdsaKeyPair) -> Result<Signature> {
        let mut sidecar_writer =
            SidecarWriter::new(self.transport.put(self.batch.header_key())?, Vec::new());
        header.write(&mut sidecar_writer)?;

        let header_signature = key
            .sign(&SystemRandom::new(), &sidecar_writer.sidecar)
            .context("failed to sign header file")?;
        Ok(header_signature)
    }

    /// Creates an avro_rs::Writer and provides it to the caller-provided
    /// function, which may then write arbitrarily many packets into it. The
    /// Avro encoding of the packet will be digested while they are written.
    /// The operation should return Ok(()) when it has finished successfully or
    /// some Err() otherwise. packet_file_writer returns the digest of all the
    /// content written by the operation.
    pub fn packet_file_writer<F>(&mut self, operation: F) -> Result<Digest>
    where
        F: FnOnce(&mut Writer<SidecarWriter<Box<dyn Write>, DigestWriter>>) -> Result<()>,
    {
        let mut writer = Writer::new(
            &self.packet_schema,
            SidecarWriter::new(
                self.transport.put(self.batch.packet_file_key())?,
                DigestWriter::new(),
            ),
        );

        operation(&mut writer)?;

        Ok(writer
            .into_inner()
            .context("failed to flush writer")?
            .sidecar
            .finish())
    }

    /// Constructs a signature structure from the provided buffers and writes it
    /// to the batch's signature file
    pub fn put_signature(&mut self, signature: &Signature) -> Result<()> {
        self.transport
            .put(self.batch.signature_key())?
            .write_all(signature.as_ref())
            .context("failed to write signature")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        idl::{IngestionDataSharePacket, IngestionHeader},
        test_utils::{
            default_facilitator_signing_public_key, default_ingestor_private_key,
            default_ingestor_public_key,
        },
        transport::LocalFileTransport,
        Error,
    };

    fn roundtrip_batch<'a>(
        aggregation_name: String,
        batch_id: Uuid,
        base_path: PathBuf,
        filenames: &[String],
        batch_writer: &mut BatchWriter<'a, IngestionHeader, IngestionDataSharePacket>,
        batch_reader: &BatchReader<'a, IngestionHeader, IngestionDataSharePacket>,
        transport: &mut LocalFileTransport,
        write_key: &EcdsaKeyPair,
        read_key: &UnparsedPublicKey<Vec<u8>>,
        keys_match: bool,
    ) {
        // These tests cheat by using IngestionHeader and
        // IngestionDataSharePacket regardless of what kind of batch it is, but
        // since BatchReader and BatchWriter are generic in H and P, this
        // exercises what we care about in this module while saving us a bit of
        // typing.
        let packets = &[
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![0u8, 1u8, 2u8, 3u8],
                encryption_key_id: "fake-key-1".to_owned(),
                r_pit: 1,
                version_configuration: Some("config-1".to_owned()),
                device_nonce: None,
            },
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![4u8, 5u8, 6u8, 7u8],
                encryption_key_id: "fake-key-2".to_owned(),
                r_pit: 2,
                version_configuration: None,
                device_nonce: Some(vec![8u8, 9u8, 10u8, 11u8]),
            },
            IngestionDataSharePacket {
                uuid: Uuid::new_v4(),
                encrypted_payload: vec![8u8, 9u8, 10u8, 11u8],
                encryption_key_id: "fake-key-3".to_owned(),
                r_pit: 3,
                version_configuration: None,
                device_nonce: None,
            },
        ];

        let packet_file_digest = batch_writer
            .packet_file_writer(|mut packet_writer| {
                packets[0].write(&mut packet_writer)?;
                packets[1].write(&mut packet_writer)?;
                packets[2].write(&mut packet_writer)?;
                Ok(())
            })
            .expect("failed to write packets");

        let header = IngestionHeader {
            batch_uuid: batch_id,
            name: aggregation_name.to_owned(),
            bins: 2,
            epsilon: 1.601,
            prime: 17,
            number_of_servers: 2,
            hamming_weight: None,
            batch_start_time: 789456123,
            batch_end_time: 789456321,
            packet_file_digest: packet_file_digest.as_ref().to_vec(),
        };

        let header_signature = batch_writer
            .put_header(&header, write_key)
            .expect("failed to write header");

        let res = batch_writer.put_signature(&header_signature);
        assert!(res.is_ok(), "failed to put signature: {:?}", res.err());

        // Verify file layout is as expected
        for extension in filenames {
            transport
                .get(&base_path.with_extension(extension))
                .expect(&format!("could not get batch file {}", extension));
        }

        let header_again = batch_reader.header(read_key);
        if !keys_match {
            assert!(
                header_again.is_err(),
                "read should fail with mismatched keys"
            );
            return;
        }
        assert!(
            header_again.is_ok(),
            "failed to read header from batch {:?}",
            header_again.err()
        );
        let header_again = header_again.unwrap();

        assert_eq!(header, header_again, "header does not match");

        let mut packet_file_reader = batch_reader
            .packet_file_reader(&header_again)
            .expect("failed to get packet file reader");

        for packet in packets {
            let packet_again = IngestionDataSharePacket::read(&mut packet_file_reader)
                .expect("failed to read packet");
            assert_eq!(packet, &packet_again, "packet does not match");
        }

        // One more read should get EOF
        match IngestionDataSharePacket::read(&mut packet_file_reader) {
            Err(Error::EofError) => (),
            v => assert!(false, "wrong error {:?}", v),
        }
    }

    #[test]
    fn roundtrip_ingestion_batch_ok() {
        roundtrip_ingestion_batch(true)
    }

    #[test]
    fn roundtrip_ingestion_batch_bad_read_key() {
        roundtrip_ingestion_batch(false)
    }

    fn roundtrip_ingestion_batch(keys_match: bool) {
        let tempdir = tempfile::TempDir::new().unwrap();
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let date = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new_ingestion(&aggregation_name, &batch_id, &date, &mut write_transport)
                .unwrap();
        let batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new_ingestion(&aggregation_name, &batch_id, &date, &mut read_transport)
                .unwrap();
        let base_path = PathBuf::new()
            .join(aggregation_name)
            .join(date.format(DATE_FORMAT).to_string())
            .join(batch_id.to_hyphenated().to_string());
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            aggregation_name.to_string(),
            batch_id,
            base_path,
            &[
                "batch".to_owned(),
                "batch.avro".to_owned(),
                "batch.sig".to_owned(),
            ],
            &mut batch_writer,
            &batch_reader,
            &mut verify_transport,
            &default_ingestor_private_key(),
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn roundtrip_validation_batch_first_ok() {
        roundtrip_validation_batch(true, true)
    }

    #[test]
    fn roundtrip_validation_batch_first_bad_read_key() {
        roundtrip_validation_batch(true, false)
    }

    #[test]
    fn roundtrip_validation_batch_second_ok() {
        roundtrip_validation_batch(false, true)
    }

    #[test]
    fn roundtrip_validation_batch_second_bad_read_key() {
        roundtrip_validation_batch(false, false)
    }

    fn roundtrip_validation_batch(is_first: bool, keys_match: bool) {
        let tempdir = tempfile::TempDir::new().unwrap();
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let date = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new_validation(
                &aggregation_name,
                &batch_id,
                &date,
                is_first,
                &mut write_transport,
            )
            .unwrap();
        let batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new_validation(
                &aggregation_name,
                &batch_id,
                &date,
                is_first,
                &mut read_transport,
            )
            .unwrap();
        let base_path = PathBuf::new()
            .join(aggregation_name)
            .join(date.format(DATE_FORMAT).to_string())
            .join(batch_id.to_hyphenated().to_string());
        let first_filenames = &[
            "validity_0".to_owned(),
            "validity_0.avro".to_owned(),
            "validity_0.sig".to_owned(),
        ];
        let second_filenames = &[
            "validity_1".to_owned(),
            "validity_1.avro".to_owned(),
            "validity_1.sig".to_owned(),
        ];
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            aggregation_name.to_string(),
            batch_id,
            base_path,
            if is_first {
                first_filenames
            } else {
                second_filenames
            },
            &mut batch_writer,
            &batch_reader,
            &mut verify_transport,
            &default_ingestor_private_key(),
            &read_key,
            keys_match,
        )
    }

    #[test]
    fn roundtrip_sum_batch_first_ok() {
        roundtrip_sum_batch(true, true)
    }

    #[test]
    fn roundtrip_sum_batch_first_bad_read_key() {
        roundtrip_sum_batch(true, false)
    }

    #[test]
    fn roundtrip_sum_batch_second_ok() {
        roundtrip_sum_batch(false, true)
    }

    #[test]
    fn roundtrip_sum_batch_second_bad_read_key() {
        roundtrip_sum_batch(false, false)
    }

    fn roundtrip_sum_batch(is_first: bool, keys_match: bool) {
        let tempdir = tempfile::TempDir::new().unwrap();
        let mut write_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut read_transport = LocalFileTransport::new(tempdir.path().to_path_buf());
        let mut verify_transport = LocalFileTransport::new(tempdir.path().to_path_buf());

        let aggregation_name = "fake-aggregation";
        let batch_id = Uuid::new_v4();
        let start = NaiveDateTime::from_timestamp(1234567890, 654321);
        let end = NaiveDateTime::from_timestamp(2234567890, 654321);

        let mut batch_writer: BatchWriter<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchWriter::new_sum(
                &aggregation_name,
                &start,
                &end,
                is_first,
                &mut write_transport,
            )
            .unwrap();
        let batch_reader: BatchReader<'_, IngestionHeader, IngestionDataSharePacket> =
            BatchReader::new_sum(
                &aggregation_name,
                &start,
                &end,
                is_first,
                &mut read_transport,
            )
            .unwrap();
        let batch_path = PathBuf::new().join(aggregation_name).join(format!(
            "{}-{}",
            start.format(DATE_FORMAT),
            end.format(DATE_FORMAT)
        ));
        let first_filenames = &[
            "sum_0".to_owned(),
            "invalid_uuid_0.avro".to_owned(),
            "sum_0.sig".to_owned(),
        ];
        let second_filenames = &[
            "sum_1".to_owned(),
            "invalid_uuid_1.avro".to_owned(),
            "sum_1.sig".to_owned(),
        ];
        let read_key = if keys_match {
            default_ingestor_public_key()
        } else {
            default_facilitator_signing_public_key()
        };
        roundtrip_batch(
            aggregation_name.to_string(),
            batch_id,
            batch_path,
            if is_first {
                first_filenames
            } else {
                second_filenames
            },
            &mut batch_writer,
            &batch_reader,
            &mut verify_transport,
            &default_ingestor_private_key(),
            &read_key,
            keys_match,
        )
    }
}
