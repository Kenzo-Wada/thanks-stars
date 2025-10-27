#[cfg(feature = "ecosystem-cargo")]
pub mod cargo;
#[cfg(feature = "ecosystem-composer")]
pub mod composer;
#[cfg(feature = "ecosystem-dart")]
pub mod dart;
#[cfg(feature = "ecosystem-deno")]
pub mod deno;
#[cfg(feature = "ecosystem-go")]
pub mod go;
#[cfg(feature = "ecosystem-gradle")]
pub mod gradle;
#[cfg(feature = "ecosystem-haskell")]
pub mod haskell;
#[cfg(feature = "ecosystem-jsr")]
pub mod jsr;
#[cfg(feature = "ecosystem-maven")]
pub mod maven;
#[cfg(feature = "ecosystem-node")]
pub mod node;
#[cfg(feature = "ecosystem-python")]
pub mod python;
#[cfg(feature = "ecosystem-renv")]
pub mod renv;
#[cfg(feature = "ecosystem-ruby")]
pub mod ruby;

#[cfg(feature = "ecosystem-cargo")]
pub use cargo::{CargoDiscoverer, CargoDiscoveryError, CommandMetadataFetcher, MetadataFetcher};
#[cfg(feature = "ecosystem-composer")]
pub use composer::{ComposerDiscoverer, ComposerDiscoveryError};
#[cfg(feature = "ecosystem-dart")]
pub use dart::{DartDiscoverer, DartDiscoveryError, HttpPubDevClient, PubDevFetcher};
#[cfg(feature = "ecosystem-deno")]
pub use deno::{DenoDiscoverer, DenoDiscoveryError};
#[cfg(feature = "ecosystem-go")]
pub use go::{GoDiscoverer, GoDiscoveryError};
#[cfg(feature = "ecosystem-gradle")]
pub use gradle::{GradleDiscoverer, GradleDiscoveryError};
#[cfg(feature = "ecosystem-haskell")]
pub use haskell::{
    HackageError, HackageFetcher, HaskellDiscoverer, HaskellDiscoveryError, HttpHackageClient,
};
#[cfg(feature = "ecosystem-jsr")]
pub use jsr::{HttpJsrClient, JsrError, JsrFetcher};
#[cfg(feature = "ecosystem-maven")]
pub use maven::{
    HttpMavenClient, MavenDependencyError, MavenDiscoverer, MavenDiscoveryError, MavenError,
    MavenFetcher, MavenProject,
};
#[cfg(feature = "ecosystem-node")]
pub use node::{NodeDiscoverer, NodeDiscoveryError};
#[cfg(feature = "ecosystem-python")]
pub use python::{HttpPyPiClient, PyPiFetcher, PythonDiscoverer, PythonDiscoveryError};
#[cfg(feature = "ecosystem-renv")]
pub use renv::{RenvDiscoverer, RenvDiscoveryError};
#[cfg(feature = "ecosystem-ruby")]
pub use ruby::{HttpRubyGemsClient, RubyDiscoverer, RubyDiscoveryError};
