//! Client side interface to the proxmox backup server
//!
//! This library implements the client side to access the backups
//! server using https.

pub mod pipe_to_stream;
mod merge_known_chunks;

mod http_client;
pub use  http_client::*;

mod task_log;
pub use task_log::*;

mod backup_reader;
pub use backup_reader::*;

mod backup_writer;
pub use backup_writer::*;

mod remote_chunk_reader;
pub use remote_chunk_reader::*;

mod pxar_backup_stream;
pub use pxar_backup_stream::*;

mod pxar_decode_writer;
pub use pxar_decode_writer::*;

mod backup_repo;
pub use backup_repo::*;

mod backup_specification;
pub use backup_specification::*;

pub mod pull;
