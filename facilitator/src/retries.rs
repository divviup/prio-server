use backoff::{retry, ExponentialBackoff};
use slog::{debug, info, Logger};
use std::{fmt::Debug, time::Duration};

/// Executes the provided action `f`, retrying with exponential backoff if the
/// error returned by `f` is deemed retryable by `is_retryable`. On success,
/// returns the value returned by `f`. On failure, returns the error returned by
/// the last attempt to call `f`. Retryable failures will be logged using the
/// provided logger.
pub(crate) fn retry_request<F, T, E, R>(logger: &Logger, f: F, is_retryable: R) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    R: FnMut(&E) -> bool,
    E: Debug,
{
    // Default ExponentialBackoff parameters are borrowed from the parameters
    // used in the GCP Go SDK[1]. AWS doesn't give us specific guidance on what
    // intervals to use, but the GCP implementation cites AWS blog posts so the
    // same parameters are probably fine for both.
    // [1] https://github.com/googleapis/gax-go/blob/fbaf9882acf3297573f3a7cb832e54c7d8f40635/v2/call_option.go#L120
    retry_request_with_params(
        logger,
        Duration::from_secs(1),
        Duration::from_secs(30),
        // We don't have explicit guidance from Google on how long to retry
        // before giving up but the Google Cloud Storage guide for retries
        // suggests 600 seconds.
        // https://cloud.google.com/storage/docs/retry-strategy#exponential-backoff
        Duration::from_secs(600),
        f,
        is_retryable,
    )
}

/// Private version of retry_request that exposes parameters for backoff. Should
/// only be used for testing. Othewise behaves identically to `retry_request`.
fn retry_request_with_params<F, T, E, R>(
    logger: &Logger,
    backoff_initial_interval: Duration,
    backoff_max_interval: Duration,
    backoff_max_elapsed: Duration,
    mut f: F,
    mut is_retryable: R,
) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    R: FnMut(&E) -> bool,
    E: Debug,
{
    let backoff = ExponentialBackoff {
        initial_interval: backoff_initial_interval,
        max_interval: backoff_max_interval,
        multiplier: 2.0,
        max_elapsed_time: Some(backoff_max_elapsed),
        ..Default::default()
    };

    retry(backoff, || {
        // Invoke the function and wrap its E into backoff::Error
        f().map_err(|error| {
            if is_retryable(&error) {
                info!(
                    logger, "encountered retryable error";
                    "error" => format!("{:?}", error),
                );
                backoff::Error::Transient(error)
            } else {
                debug!(logger, "encountered non-retryable error");
                backoff::Error::Permanent(error)
            }
        })
    })
    // Unwrap the backoff::Error to get the E back and let the caller wrap that
    // in whatever they want
    .map_err(|e| match e {
        backoff::Error::Permanent(inner) => inner,
        backoff::Error::Transient(inner) => inner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::setup_test_logging;

    #[test]
    fn success() {
        let logger = setup_test_logging();
        let mut counter = 0;
        let f = || -> Result<(), bool> {
            counter += 1;
            Ok(())
        };

        retry_request_with_params(
            &logger,
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(10),
            f,
            |_| false,
        )
        .unwrap();
        assert_eq!(counter, 1);
    }

    #[test]
    fn retryable_failure() {
        let logger = setup_test_logging();
        let mut counter = 0;
        let f = || -> Result<(), bool> {
            counter += 1;
            if counter == 1 {
                Err(false)
            } else {
                Ok(())
            }
        };

        retry_request_with_params(
            &logger,
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(30),
            f,
            |_| true,
        )
        .unwrap();
        assert!(counter > 1);
    }

    #[test]
    fn retryable_failure_exhaust_max_elapsed() {
        let logger = setup_test_logging();
        let mut counter = 0;
        let f = || -> std::result::Result<(), bool> {
            counter += 1;
            Err(false)
        };

        retry_request_with_params(
            &logger,
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(30),
            f,
            |_| true,
        )
        .unwrap_err();
        assert!(counter >= 2);
    }

    #[test]
    fn unretryable_failure() {
        let logger = setup_test_logging();
        let mut counter = 0;
        let f = || -> std::result::Result<(), bool> {
            counter += 1;
            Err(false)
        };

        retry_request_with_params(
            &logger,
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(30),
            f,
            |_| false,
        )
        .unwrap_err();
        assert_eq!(counter, 1);
    }
}
