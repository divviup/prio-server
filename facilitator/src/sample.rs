use crate::idl::IngestionHeader;
use crate::transport::Transport;
use crate::Error;
use libprio_rs::encrypt::PrivateKey;
use libprio_rs::finite_field::MODULUS;
//use rand::{thread_rng, Rng};
use std::path::PathBuf;
use uuid::Uuid;

pub fn generate_ingestion_sample(
    transport: Box<dyn Transport>,
    batch_uuid: Uuid,
    aggregation_name: String,
    date: String,
    _pha_key: &PrivateKey,
    _facilitator_key: &PrivateKey,
    _dim: usize,
    _packet_count: usize,
    bins: i32,
    epsilon: f64,
    batch_start_time: i64,
    batch_end_time: i64,
) -> Result<(), Error> {
    // Write ingestion header
    let ingestion_header = IngestionHeader {
        batch_uuid: batch_uuid,
        name: aggregation_name.to_owned(),
        bins: bins,
        epsilon: epsilon,
        prime: MODULUS as i64,
        number_of_servers: 2,
        hamming_weight: None,
        batch_start_time: batch_start_time,
        batch_end_time: batch_end_time,
    };

    let mut writer = transport.put(
        &PathBuf::from(aggregation_name)
            .join(date)
            .join(format!("{}.batch", batch_uuid)),
    )?;
    ingestion_header.write(&mut writer)?;

    /*
        // Generate random data packets and write into data share packets
        let mut rng = thread_rng();

        for _ in 0..packet_count {
            let data = (0..dim)
                .map(|_| Field::from(rng.gen_range(0, 2)))
                .collect::<Vec<Field>>();
        }

        // Sign files and write ingestion signature
    */
    Ok(())
}
