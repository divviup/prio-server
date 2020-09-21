use crate::{
    idl::{Header, Packet},
    transport::Transport,
    DigestWriter, Error, SidecarWriter, DATE_FORMAT,
};
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
    ) -> Result<Self, Error> {
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
    ) -> Result<Self, Error> {
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
    ) -> Result<Self, Error> {
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

    fn new(batch: Batch, transport: &'a mut dyn Transport) -> Result<Self, Error>;
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
    fn new(batch: Batch, transport: &'a mut dyn Transport) -> Result<Self, Error> {
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
    pub fn header(&self, key: &UnparsedPublicKey<Vec<u8>>) -> Result<H, Error> {
        let mut signature = Vec::new();
        self.transport
            .get(self.batch.signature_key())?
            .read_to_end(&mut signature)
            .map_err(|e| Error::IoError("failed to read signature".to_owned(), e))?;

        let mut header_buf = Vec::new();
        self.transport
            .get(self.batch.header_key())?
            .read_to_end(&mut header_buf)
            .map_err(|e| Error::IoError("failed to read header from transport".to_owned(), e))?;

        key.verify(&header_buf, &signature).map_err(|e| {
            Error::CryptographyError("invalid signature on header".to_owned(), None, Some(e))
        })?;

        H::read(Cursor::new(header_buf))
    }

    /// Return an avro_rs::Reader that yields the packets in the packet file,
    /// but only if the whole file's digest matches the packet_file_digest field
    /// in the provided header. The header is assumed to be trusted.
    pub fn packet_file_reader(&self, header: &H) -> Result<Reader<Cursor<Vec<u8>>>, Error> {
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
            .map_err(|e| Error::IoError("failed to load packet file".to_owned(), e))?;

        // ... then verify the digest over it ...
        if header.packet_file_digest().as_slice() != sidecar_writer.sidecar.finish().as_ref() {
            return Err(Error::CryptographyError(
                "packet file digest does not match header".to_owned(),
                None,
                None,
            ));
        }

        // ... then return a packet reader.
        Reader::with_schema(&self.packet_schema, Cursor::new(sidecar_writer.writer))
            .map_err(|e| Error::AvroError("failed to create Avro reader for packets".to_owned(), e))
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
    fn new(batch: Batch, transport: &'a mut dyn Transport) -> Result<Self, Error> {
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
    pub fn put_header(&mut self, header: &H, key: &EcdsaKeyPair) -> Result<Signature, Error> {
        let mut sidecar_writer =
            SidecarWriter::new(self.transport.put(self.batch.header_key())?, Vec::new());
        header.write(&mut sidecar_writer)?;

        let header_signature = key
            .sign(&SystemRandom::new(), &sidecar_writer.sidecar)
            .map_err(|e| {
                Error::CryptographyError("failed to sign header file".to_owned(), None, Some(e))
            })?;
        Ok(header_signature)
    }

    /// Creates an avro_rs::Writer and provides it to the caller-provided
    /// function, which may then write arbitrarily many packets into it. The
    /// Avro encoding of the packet will be digested while they are written.
    /// The operation should return Ok(()) when it has finished successfully or
    /// some Err() otherwise. packet_file_writer returns the digest of all the
    /// content written by the operation.
    pub fn packet_file_writer<F>(&mut self, operation: F) -> Result<Digest, Error>
    where
        F: FnOnce(&mut Writer<SidecarWriter<Box<dyn Write>, DigestWriter>>) -> Result<(), Error>,
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
            .map_err(|e| Error::AvroError("failed to flush writer".to_owned(), e))?
            .sidecar
            .finish())
    }

    /// Constructs a signature structure from the provided buffers and writes it
    /// to the batch's signature file
    pub fn put_signature(&mut self, signature: &Signature) -> Result<(), Error> {
        self.transport
            .put(self.batch.signature_key())?
            .write_all(signature.as_ref())
            .map_err(|e| Error::IoError("failed to write signature".to_owned(), e))
    }
}
