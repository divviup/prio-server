use anyhow::Result;
use chrono::{DateTime, Utc};
use std::iter;
use uuid::Uuid;

/// Splits a path in two parts, containing the base path with filename and its file extensions. The
/// extension part will include the leading `.`, and unlike
/// Returns the entire path and filename, but without any file extensions (compound or otherwise).
pub fn split_path_extensions(path: &str) -> (&str, &str) {
    path.rmatch_indices(['.', '/'].as_ref())
        // Isolate the last path component by stopping at the first (right-most) '/'
        .take_while(|(_, c)| *c != "/")
        // Pick the last (left-most) '.' as the end of the prefix
        .last()
        .map_or((path, ""), |(i, c)| {
            assert_eq!(c, ".");
            path.split_at(i)
        })
}

#[derive(Debug)]
pub struct IngestionBatchFileMetadata {
    path_prefix: String,

    aggregation_name: String,
    timestamp: DateTime<Utc>,
    uuid: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestionBatchParseError {
    #[error("component `{0}` is missing")]
    MissingComponent(&'static str),
    #[error("component `{0}` has an invalid format")]
    InvalidComponent(&'static str),
    #[error("extra trailing components at the end of path: `{0}`")]
    ExtraComponents(String),
    #[error("failed to parse timestamp (at component {})", .field.unwrap_or("[unknown]"))]
    InvalidTimestamp {
        #[source]
        source: chrono::format::ParseError,
        field: Option<&'static str>,
    },
    #[error("invalid uuid")]
    InvalidUuid(#[source] uuid::Error),
}

impl IngestionBatchFileMetadata {
    pub fn parse_from_prefix(path_prefix: &str) -> Result<Self, IngestionBatchParseError> {
        use chrono::format::{self, Item, Numeric, Pad, Parsed};
        use IngestionBatchParseError::*;

        fn text_component<'a>(
            components: &mut impl Iterator<Item = &'a str>,
            component_label: &'static str,
        ) -> Result<&'a str, IngestionBatchParseError> {
            components.next().ok_or(MissingComponent(component_label))
        }

        fn datetime_component<'a>(
            components: &mut impl Iterator<Item = &'a str>,
            partial_datetime: &mut Parsed,
            numeric_field_type: Numeric,
            component_label: &'static str,
        ) -> Result<(), IngestionBatchParseError> {
            format::parse(
                partial_datetime,
                text_component(components, component_label)?,
                iter::once(Item::Numeric(numeric_field_type, Pad::None)),
            )
            .map_err(|e| InvalidTimestamp {
                source: e,
                field: Some(component_label),
            })
        };

        // Path format: `{aggregation_id}/YYYY/mm/dd/HH/MM/{batch_id}[/invalid trailing garbage]`
        let mut components = path_prefix.splitn(8, '/');
        let c = &mut components;

        let aggregation_id = text_component(c, "aggregation_id")?.to_owned();

        let mut partial_datetime = Parsed::new();
        let dt = &mut partial_datetime;
        datetime_component(c, dt, Numeric::Year, "year")?;
        datetime_component(c, dt, Numeric::Month, "month")?;
        datetime_component(c, dt, Numeric::Day, "day")?;
        datetime_component(c, dt, Numeric::Hour, "hour")?;
        datetime_component(c, dt, Numeric::Minute, "minute")?;
        let timestamp = partial_datetime
            .to_datetime_with_timezone(&Utc)
            .map_err(|e| InvalidTimestamp {
                source: e,
                field: None,
            })?;

        let batch_id = text_component(c, "batch_id")?
            .parse()
            .map_err(InvalidUuid)?;

        if let Some(remainder) = components.next() {
            assert!(components.next().is_none());
            return Err(ExtraComponents(remainder.to_owned()));
        }

        Ok(IngestionBatchFileMetadata {
            path_prefix: path_prefix.to_owned(),
            aggregation_name: aggregation_id,
            timestamp,
            uuid: batch_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use chrono::TimeZone;

    #[test]
    fn test_split_path_extensions() {
        let expected_key = "fake-aggregation/2020/10/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b";
        for &(path, expected_extension) in &[
            (
                "fake-aggregation/2020/10/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b.batch",
                ".batch",
            ),
            (
                "fake-aggregation/2020/10/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b.batch.avro",
                ".batch.avro",
            ),
            (
                "fake-aggregation/2020/10/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b.batch.sig",
                ".batch.sig",
            ),
        ] {
            assert_eq!(
                split_path_extensions(path),
                (expected_key, expected_extension)
            );
        }

        assert_eq!(
            split_path_extensions("loose-filename.single-extension"),
            ("loose-filename", ".single-extension")
        );
        assert_eq!(
            split_path_extensions("loose-filename.with.extension"),
            ("loose-filename", ".with.extension")
        );
        assert_eq!(
            split_path_extensions("path.with.dot/more.dots/filename.with.extension"),
            ("path.with.dot/more.dots/filename", ".with.extension")
        );
        assert_eq!(
            split_path_extensions("path.with.dot/more.dots/filename-with-no-extension"),
            ("path.with.dot/more.dots/filename-with-no-extension", "")
        );
    }

    #[test]
    fn test_ingestion_metadata_parse() {
        let prefix = "fake-aggregation/2020/05/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b";
        let parsed = IngestionBatchFileMetadata::parse_from_prefix(prefix).unwrap();
        assert_eq!(parsed.path_prefix, prefix);
        assert_eq!(parsed.aggregation_name, "fake-aggregation");
        assert_eq!(parsed.timestamp, Utc.ymd(2020, 5, 10).and_hms(20, 30, 0));
        assert_eq!(
            parsed.uuid,
            "6891ce17-623f-41f7-9c1d-20fc2f98248b".parse().unwrap()
        );

        // Test a few error cases
        let prefix = "oops/fake-aggregation/2020/05/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b";
        assert_matches!(
            IngestionBatchFileMetadata::parse_from_prefix(prefix),
            Err(IngestionBatchParseError::InvalidTimestamp {
                source: _,
                field: Some("year"),
            })
        );

        let prefix = "fake-aggregation/2020/05/10/20/30";
        assert_matches!(
            IngestionBatchFileMetadata::parse_from_prefix(prefix),
            Err(IngestionBatchParseError::MissingComponent("batch_id"))
        );

        let prefix =
            "fake-aggregation/2020/05/10/20/30/6891ce17-623f-41f7-9c1d-20fc2f98248b/trailing/stuff";
        assert_matches!(
            IngestionBatchFileMetadata::parse_from_prefix(prefix),
            Err(IngestionBatchParseError::ExtraComponents(extra)) => {
                assert_eq!(extra, "trailing/stuff".to_string());
            }
        );
    }
}
