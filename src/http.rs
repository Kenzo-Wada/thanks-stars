use std::sync::LazyLock;

use reqwest::blocking::Client;

static SHARED_CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

/// Return a clone of the globally shared blocking [`Client`].
///
/// Building a [`Client`] is relatively expensive due to TLS initialization.
/// Sharing a single instance lets individual fetchers reuse the same
/// connection pool without paying the construction cost repeatedly.
pub fn shared_client() -> Client {
    SHARED_CLIENT.clone()
}
