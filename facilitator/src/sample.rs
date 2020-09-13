use crate::idl::{ingestion_data_share_packet_schema, IngestionDataSharePacket, IngestionHeader};
use crate::transport::Transport;
use crate::Error;
use avro_rs::Writer;
use libprio_rs::client::Client;
use libprio_rs::encrypt::{PrivateKey, PublicKey};
use libprio_rs::finite_field::{Field, MODULUS};
use libprio_rs::server::Server;
use rand::{thread_rng, Rng};
use std::path::PathBuf;
use uuid::Uuid;

pub const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";
pub const DEFAULT_PHA_PRIVATE_KEY: &str =
    "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9Rq\
    Zx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==";
pub const DEFAULT_FACILITATOR_PRIVATE_KEY: &str =
    "BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g\
    /WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==";

fn write_ingestion_header(
    transport: &mut dyn Transport,
    path: &PathBuf,
    header: &IngestionHeader,
) -> Result<(), Error> {
    let mut header_writer = transport.put(&path)?;
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
    dim: i32,
    packet_count: usize,
    epsilon: f64,
    batch_start_time: i64,
    batch_end_time: i64,
) -> Result<(), Error> {
    if dim <= 0 {
        return Err(Error::IllegalArgumentError(
            "dimension must be natural number".to_string(),
        ));
    }

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

    let header_path = PathBuf::from(aggregation_name.to_owned())
        .join(date.to_owned())
        .join(format!("{}.batch", batch_uuid));

    write_ingestion_header(pha_transport, &header_path, &ingestion_header)?;
    write_ingestion_header(facilitator_transport, &header_path, &ingestion_header)?;

    // Generate random data packets and write into data share packets
    let mut rng = thread_rng();

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

    let packet_file_path = PathBuf::from(aggregation_name.to_owned())
        .join(date.to_owned())
        .join(format!("{}.batch.avro", batch_uuid));

    let schema = ingestion_data_share_packet_schema().unwrap();

    let mut pha_packet_transport_writer = pha_transport.put(&packet_file_path)?;
    let mut pha_packet_writer = Writer::new(&schema, &mut pha_packet_transport_writer);

    let mut facilitator_packet_transport_writer =
        &mut facilitator_transport.put(&packet_file_path)?;
    let mut facilitator_packet_writer =
        Writer::new(&schema, &mut facilitator_packet_transport_writer);

    // We need an instance of a libprio server to pick an r_pit, which seems odd
    // because that value is essentially random and must be agreed upon by the
    // PHA and facilitator so I suspect the client must generate it.
    // TODO(timg) expose this in libprio client
    let fake_server = Server::new(dim as usize, true, pha_key.clone());

    for _ in 0..packet_count {
        // Generate random bit vector
        let data = (0..dim)
            .map(|_| Field::from(rng.gen_range(0, 2)))
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

        let facilitator_packet = IngestionDataSharePacket {
            uuid: Uuid::new_v4(),
            encrypted_payload: facilitator_share,
            encryption_key_id: "facilitator-fake-key-1".to_owned(),
            r_pit: u32::from(r_pit) as i64,
            version_configuration: Some("config-1".to_owned()),
            device_nonce: None,
        };

        facilitator_packet.write(&mut facilitator_packet_writer)?;
    }

    // TODO(timg) Sign files and write ingestion signature. Not yet clear what
    // algorithm, key size, format, etc. to use.

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FileTransport;

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
