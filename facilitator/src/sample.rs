use crate::{
    idl::{
        ingestion_data_share_packet_schema, IngestionDataSharePacket, IngestionHeader,
        IngestionSignature,
    },
    ingestion::Batch,
    transport::Transport,
    Error,
};
use avro_rs::Writer;
use libprio_rs::{
    client::Client,
    encrypt::{PrivateKey, PublicKey},
    finite_field::{Field, MODULUS},
    server::Server,
};
use rand::{thread_rng, Rng};
use ring::{
    rand::SystemRandom,
    signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use std::path::Path;
use uuid::Uuid;

fn write_ingestion_header(
    transport: &mut dyn Transport,
    path: &Path,
    header: &IngestionHeader,
) -> Result<(), Error> {
    let mut header_writer = transport.put(path)?;
    header.write(&mut header_writer)
}

pub fn generate_ingestion_sample(
    pha_transport: &mut dyn Transport,
    facilitator_transport: &mut dyn Transport,
    batch_uuid: Uuid,
    aggregation_name: String,
    date: String,
    pha_key: &PrivateKey,
    facilitator_key: &PrivateKey,
    ingestor_key: &[u8],
    dim: i32,
    packet_count: usize,
    epsilon: f64,
    batch_start_time: i64,
    batch_end_time: i64,
) -> Result<(), Error> {
    if dim <= 0 {
        return Err(Error::IllegalArgumentError(
            "dimension must be natural number".to_owned(),
        ));
    }

    let ingestor_key_pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, ingestor_key).map_err(|e| {
            Error::CryptographyError(
                "failed to parse ingestor key pair".to_owned(),
                Some(e),
                None,
            )
        })?;
    let rng = SystemRandom::new();

    let ingestion_batch = Batch::new_ingestion(aggregation_name.to_owned(), batch_uuid, date);

    // Write ingestion header
    let ingestion_header = IngestionHeader {
        batch_uuid: batch_uuid,
        name: aggregation_name.to_owned(),
        bins: dim,
        epsilon: epsilon,
        prime: MODULUS as i64,
        number_of_servers: 2,
        hamming_weight: None,
        batch_start_time: batch_start_time,
        batch_end_time: batch_end_time,
    };

    write_ingestion_header(
        pha_transport,
        ingestion_batch.header_key(),
        &ingestion_header,
    )?;
    write_ingestion_header(
        facilitator_transport,
        ingestion_batch.header_key(),
        &ingestion_header,
    )?;

    let mut signature_message = Vec::new();
    ingestion_header.write(&mut signature_message)?;
    let header_signature = ingestor_key_pair
        .sign(&rng, &signature_message)
        .map_err(|e| {
            Error::CryptographyError("failed to sign ingestion header".to_owned(), None, Some(e))
        })?;

    // Generate random data packets and write into data share packets
    let mut thread_rng = thread_rng();

    let mut client = match Client::new(
        // usize is probably bigger than i32 and we have checked that dim is
        // positive so this is safe
        dim as usize,
        PublicKey::from(pha_key),
        PublicKey::from(facilitator_key),
    ) {
        Some(c) => c,
        None => {
            return Err(Error::LibPrioError(
                "failed to create client (bad dimension parameter?)".to_owned(),
                None,
            ))
        }
    };

    let schema = ingestion_data_share_packet_schema();

    // ring::signature does not provide a means of feeding chunks of a message
    // into a signer. You must instead provide the entire message at once
    // (https://github.com/briansmith/ring/issues/253).
    let mut pha_signature_message = Vec::new();
    let mut pha_packet_transport_writer = pha_transport.put(ingestion_batch.packet_file_key())?;
    let mut pha_packet_writer = Writer::new(&schema, &mut pha_packet_transport_writer);
    let mut pha_packet_signature_writer = Writer::new(&schema, &mut pha_signature_message);

    let mut facilitator_signature_message = Vec::new();
    let mut facilitator_packet_transport_writer =
        &mut facilitator_transport.put(ingestion_batch.packet_file_key())?;
    let mut facilitator_packet_writer =
        Writer::new(&schema, &mut facilitator_packet_transport_writer);
    let mut facilitator_packet_signature_writer =
        Writer::new(&schema, &mut facilitator_signature_message);

    // We need an instance of a libprio server to pick an r_pit, which seems odd
    // because that value is essentially random and must be agreed upon by the
    // PHA and facilitator so I suspect the client must generate it.
    // TODO(timg) expose this in libprio client
    let fake_server = Server::new(dim as usize, true, pha_key.clone());

    for _ in 0..packet_count {
        // Generate random bit vector
        let data = (0..dim)
            .map(|_| Field::from(thread_rng.gen_range(0, 2)))
            .collect::<Vec<Field>>();

        let (pha_share, facilitator_share) = client
            .encode_simple(&data)
            .map_err(|e| Error::LibPrioError("failed to encode data".to_owned(), Some(e)))?;

        let r_pit = fake_server.choose_eval_at();

        // TODO(timg): we encode the shares here directly as bytes, but other
        // implementations encode them in Base64 first. The B64 encoding is
        // unnecessary and introduces a ~33% overhead so I feel we should not
        // bother with it.
        let pha_packet = IngestionDataSharePacket {
            uuid: Uuid::new_v4(),
            encrypted_payload: pha_share,
            encryption_key_id: "pha-fake-key-1".to_owned(),
            r_pit: u32::from(r_pit) as i64,
            version_configuration: Some("config-1".to_owned()),
            device_nonce: None,
        };

        pha_packet.write(&mut pha_packet_writer)?;
        pha_packet.write(&mut pha_packet_signature_writer)?;

        let facilitator_packet = IngestionDataSharePacket {
            uuid: Uuid::new_v4(),
            encrypted_payload: facilitator_share,
            encryption_key_id: "facilitator-fake-key-1".to_owned(),
            r_pit: u32::from(r_pit) as i64,
            version_configuration: Some("config-1".to_owned()),
            device_nonce: None,
        };

        facilitator_packet.write(&mut facilitator_packet_writer)?;
        facilitator_packet.write(&mut facilitator_packet_signature_writer)?;
    }

    // Sign the PHA's packet file, then construct signature Avro message and PUT
    // it into PHA transport.
    let pha_packet_file_signature = ingestor_key_pair
        .sign(&rng, &pha_signature_message)
        .map_err(|e| {
            Error::CryptographyError("failed to sign PHA packet file".to_owned(), None, Some(e))
        })?;
    let mut signature_writer = pha_transport.put(ingestion_batch.signature_path())?;
    IngestionSignature {
        batch_header_signature: header_signature.as_ref().to_vec(),
        signature_of_packets: pha_packet_file_signature.as_ref().to_vec(),
    }
    .write(&mut signature_writer)?;

    // Same thing for the facilitator.
    let facilitator_packet_file_signature = ingestor_key_pair
        .sign(&rng, &facilitator_signature_message)
        .map_err(|e| {
            Error::CryptographyError(
                "failed to sign facilitator packet file".to_owned(),
                None,
                Some(e),
            )
        })?;
    let mut signature_writer = facilitator_transport.put(ingestion_batch.signature_path())?;
    IngestionSignature {
        batch_header_signature: header_signature.as_ref().to_vec(),
        signature_of_packets: facilitator_packet_file_signature.as_ref().to_vec(),
    }
    .write(&mut signature_writer)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        default_ingestor_private_key, transport::FileTransport, DEFAULT_FACILITATOR_PRIVATE_KEY,
        DEFAULT_PHA_PRIVATE_KEY,
    };
    use std::path::PathBuf;

    #[test]
    fn write_sample() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let batch_uuid = Uuid::new_v4();
        let mut pha_transport = FileTransport::new(tempdir.path().to_path_buf().join("pha"));
        let mut facilitator_transport =
            FileTransport::new(tempdir.path().to_path_buf().join("pha"));

        let res = generate_ingestion_sample(
            &mut pha_transport,
            &mut facilitator_transport,
            batch_uuid,
            "fake-aggregation".to_owned(),
            "fake-date".to_owned(),
            &PrivateKey::from_base64(DEFAULT_PHA_PRIVATE_KEY).unwrap(),
            &PrivateKey::from_base64(DEFAULT_FACILITATOR_PRIVATE_KEY).unwrap(),
            &default_ingestor_private_key(),
            10,
            10,
            0.11,
            100,
            100,
        );
        assert!(res.is_ok(), "error writing sample data {:?}", res.err());
        let mut expected_path =
            PathBuf::from("fake-aggregation/fake-date").join(batch_uuid.to_string());

        let transports = &[pha_transport, facilitator_transport];
        for transport in transports {
            expected_path.set_extension("batch");
            let reader = transport.get(&expected_path);
            assert!(res.is_ok(), "error reading header back {:?}", res.err());

            let parsed_header = IngestionHeader::read(reader.unwrap());
            assert!(
                parsed_header.is_ok(),
                "error parsing header {:?}",
                res.err()
            );
            assert_eq!(
                parsed_header.unwrap(),
                IngestionHeader {
                    batch_uuid: batch_uuid,
                    name: "fake-aggregation".to_owned(),
                    bins: 10,
                    epsilon: 0.11,
                    prime: MODULUS as i64,
                    number_of_servers: 2,
                    hamming_weight: None,
                    batch_start_time: 100,
                    batch_end_time: 100,
                }
            );
        }
    }
}
