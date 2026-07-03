//! Record-segment transport (搬运): copy recorded segments to remote storage
//! via a pluggable [`backend::StorageBackend`] (FTP / SMB now, S3 later),
//! configured through the DB/REST API and driven by a background worker.

pub mod api;
mod backend;
pub mod config;
mod ftp;
#[cfg(feature = "smb")]
mod smb;
mod worker;

pub use worker::spawn_worker;
