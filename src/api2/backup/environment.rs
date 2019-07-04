use failure::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

use serde_json::Value;

use proxmox::tools;

use crate::api_schema::router::{RpcEnvironment, RpcEnvironmentType};
use crate::server::WorkerTask;
use crate::backup::*;
use crate::server::formatter::*;
use hyper::{Body, Response};

struct UploadStatistic {
    count: u64,
    size: u64,
    compressed_size: u64,
    duplicates: u64,
}

impl UploadStatistic {
    fn new() -> Self {
        Self {
            count: 0,
            size: 0,
            compressed_size: 0,
            duplicates: 0,
        }
    }
}

struct DynamicWriterState {
    name: String,
    index: DynamicIndexWriter,
    offset: u64,
    chunk_count: u64,
    upload_stat: UploadStatistic,
}

struct FixedWriterState {
    name: String,
    index: FixedIndexWriter,
    size: usize,
    chunk_size: u32,
    chunk_count: u64,
    small_chunk_count: usize, // allow 0..1 small chunks (last chunk may be smaller)
    upload_stat: UploadStatistic,
}

struct SharedBackupState {
    finished: bool,
    uid_counter: usize,
    file_counter: usize, // sucessfully uploaded files
    dynamic_writers: HashMap<usize, DynamicWriterState>,
    fixed_writers: HashMap<usize, FixedWriterState>,
    known_chunks: HashMap<[u8;32], u32>,
}

impl SharedBackupState {

    // Raise error if finished flag is set
    fn ensure_unfinished(&self) -> Result<(), Error> {
        if self.finished {
            bail!("backup already marked as finished.");
        }
        Ok(())
    }

    // Get an unique integer ID
    pub fn next_uid(&mut self) -> usize {
        self.uid_counter += 1;
        self.uid_counter
    }
}


/// `RpcEnvironmet` implementation for backup service
#[derive(Clone)]
pub struct BackupEnvironment {
    env_type: RpcEnvironmentType,
    result_attributes: HashMap<String, Value>,
    user: String,
    pub debug: bool,
    pub formatter: &'static OutputFormatter,
    pub worker: Arc<WorkerTask>,
    pub datastore: Arc<DataStore>,
    pub backup_dir: BackupDir,
    pub last_backup: Option<BackupInfo>,
    state: Arc<Mutex<SharedBackupState>>
}

impl BackupEnvironment {
    pub fn new(
        env_type: RpcEnvironmentType,
        user: String,
        worker: Arc<WorkerTask>,
        datastore: Arc<DataStore>,
        backup_dir: BackupDir,
    ) -> Self {

        let state = SharedBackupState {
            finished: false,
            uid_counter: 0,
            file_counter: 0,
            dynamic_writers: HashMap::new(),
            fixed_writers: HashMap::new(),
            known_chunks: HashMap::new(),
        };

        Self {
            result_attributes: HashMap::new(),
            env_type,
            user,
            worker,
            datastore,
            debug: false,
            formatter: &JSON_FORMATTER,
            backup_dir,
            last_backup: None,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Register a Chunk with associated length.
    ///
    /// We do not fully trust clients, so a client may only use registered
    /// chunks. Please use this method to register chunks from previous backups.
    pub fn register_chunk(&self, digest: [u8; 32], length: u32) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        state.known_chunks.insert(digest, length);

        Ok(())
    }

    /// Register fixed length chunks after upload.
    ///
    /// Like `register_chunk()`, but additionally record statistics for
    /// the fixed index writer.
    pub fn register_fixed_chunk(
        &self,
        wid: usize,
        digest: [u8; 32],
        size: u32,
        compressed_size: u32,
        is_duplicate: bool,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.fixed_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("fixed writer '{}' not registered", wid),
        };

        if size > data.chunk_size {
            bail!("fixed writer '{}' - got large chunk ({} > {}", data.name, size, data.chunk_size);
        } else if size < data.chunk_size {
            data.small_chunk_count += 1;
            if data.small_chunk_count > 1 {
                bail!("fixed writer '{}' - detected multiple end chunks (chunk size too small)");
            }
        }

        // record statistics
        data.upload_stat.count += 1;
        data.upload_stat.size += size as u64;
        data.upload_stat.compressed_size += compressed_size as u64;
        if is_duplicate { data.upload_stat.duplicates += 1; }

        // register chunk
        state.known_chunks.insert(digest, size);

        Ok(())
    }

    /// Register dynamic length chunks after upload.
    ///
    /// Like `register_chunk()`, but additionally record statistics for
    /// the dynamic index writer.
    pub fn register_dynamic_chunk(
        &self,
        wid: usize,
        digest: [u8; 32],
        size: u32,
        compressed_size: u32,
        is_duplicate: bool,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.dynamic_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        // record statistics
        data.upload_stat.count += 1;
        data.upload_stat.size += size as u64;
        data.upload_stat.compressed_size += compressed_size as u64;
        if is_duplicate { data.upload_stat.duplicates += 1; }

        // register chunk
        state.known_chunks.insert(digest, size);

        Ok(())
    }

    pub fn lookup_chunk(&self, digest: &[u8; 32]) -> Option<u32> {
        let state = self.state.lock().unwrap();

        match state.known_chunks.get(digest) {
            Some(len) => Some(*len),
            None => None,
        }
    }

    /// Store the writer with an unique ID
    pub fn register_dynamic_writer(&self, index: DynamicIndexWriter, name: String) -> Result<usize, Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let uid = state.next_uid();

        state.dynamic_writers.insert(uid, DynamicWriterState {
            index, name, offset: 0, chunk_count: 0, upload_stat: UploadStatistic::new(),
        });

        Ok(uid)
    }

    /// Store the writer with an unique ID
    pub fn register_fixed_writer(&self, index: FixedIndexWriter, name: String, size: usize, chunk_size: u32) -> Result<usize, Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let uid = state.next_uid();

        state.fixed_writers.insert(uid, FixedWriterState {
            index, name, chunk_count: 0, size, chunk_size, small_chunk_count: 0, upload_stat: UploadStatistic::new(),
        });

        Ok(uid)
    }

    /// Append chunk to dynamic writer
    pub fn dynamic_writer_append_chunk(&self, wid: usize, offset: u64, size: u32, digest: &[u8; 32]) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.dynamic_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };


        if data.offset != offset {
            bail!("dynamic writer '{}' append chunk failed - got strange chunk offset ({} != {})",
                  data.name, data.offset, offset);
        }

        data.offset += size as u64;
        data.chunk_count += 1;

        data.index.add_chunk(data.offset, digest)?;

        Ok(())
    }

    /// Append chunk to fixed writer
    pub fn fixed_writer_append_chunk(&self, wid: usize, offset: u64, size: u32, digest: &[u8; 32]) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.fixed_writers.get_mut(&wid) {
            Some(data) => data,
            None => bail!("fixed writer '{}' not registered", wid),
        };

        let end = (offset as usize) + (size as usize);
        let idx = data.index.check_chunk_alignment(end, size as usize)?;

        data.chunk_count += 1;

        data.index.add_digest(idx, digest)?;

        Ok(())
    }

    fn log_upload_stat(&self, archive_name:  &str, csum: &[u8; 32], uuid: &[u8; 16], size: u64, chunk_count: u64, upload_stat: &UploadStatistic) {
        self.log(format!("Upload statistics for '{}'", archive_name));
        self.log(format!("UUID: {}", tools::digest_to_hex(uuid)));
        self.log(format!("Checksum: {}", tools::digest_to_hex(csum)));
        self.log(format!("Size: {}", size));
        self.log(format!("Chunk count: {}", chunk_count));

        if size == 0 || chunk_count == 0 {
            return;
        }

        self.log(format!("Upload size: {} ({}%)", upload_stat.size, (upload_stat.size*100)/size));

        let client_side_duplicates = chunk_count - upload_stat.count;
        let server_side_duplicates = upload_stat.duplicates;

        if (client_side_duplicates + server_side_duplicates) > 0 {
            let per = (client_side_duplicates + server_side_duplicates)*100/chunk_count;
            self.log(format!("Duplicates: {}+{} ({}%)", client_side_duplicates, server_side_duplicates, per));
        }

        if upload_stat.size > 0 {
            self.log(format!("Compression: {}%", (upload_stat.compressed_size*100)/upload_stat.size));
        }
    }

    /// Close dynamic writer
    pub fn dynamic_writer_close(&self, wid: usize, chunk_count: u64, size: u64) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.dynamic_writers.remove(&wid) {
            Some(data) => data,
            None => bail!("dynamic writer '{}' not registered", wid),
        };

        if data.chunk_count != chunk_count {
            bail!("dynamic writer '{}' close failed - unexpected chunk count ({} != {})", data.name, data.chunk_count, chunk_count);
        }

        if data.offset != size {
            bail!("dynamic writer '{}' close failed - unexpected file size ({} != {})", data.name, data.offset, size);
        }

        let uuid = data.index.uuid;

        let csum = data.index.close()?;

        self.log_upload_stat(&data.name, &csum, &uuid, size, chunk_count, &data.upload_stat);

        state.file_counter += 1;

        Ok(())
    }

    /// Close fixed writer
    pub fn fixed_writer_close(&self, wid: usize, chunk_count: u64, size: u64) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();

        state.ensure_unfinished()?;

        let mut data = match state.fixed_writers.remove(&wid) {
            Some(data) => data,
            None => bail!("fixed writer '{}' not registered", wid),
        };

        if data.chunk_count != chunk_count {
            bail!("fixed writer '{}' close failed - received wrong number of chunk ({} != {})", data.name, data.chunk_count, chunk_count);
        }

        let expected_count = data.index.index_length();

        if chunk_count != (expected_count as u64) {
            bail!("fixed writer '{}' close failed - unexpected chunk count ({} != {})", data.name, expected_count, chunk_count);
        }

        if size != (data.size as u64) {
            bail!("fixed writer '{}' close failed - unexpected file size ({} != {})", data.name, data.size, size);
        }

        let uuid = data.index.uuid;

        let csum = data.index.close()?;

        self.log_upload_stat(&data.name, &csum, &uuid, size, chunk_count, &data.upload_stat);

        state.file_counter += 1;

        Ok(())
    }

    pub fn add_blob(&self, file_name: &str, data: Vec<u8>) -> Result<(), Error> {

        let mut path = self.datastore.base_path();
        path.push(self.backup_dir.relative_path());
        path.push(file_name);

        let blob_len = data.len();
        let orig_len = data.len(); // fixme:

        let mut blob = DataBlob::from_raw(data)?;
        // always comput CRC at server side
        blob.set_crc(blob.compute_crc());

        let raw_data = blob.raw_data();
        crate::tools::file_set_contents(&path, raw_data, None)?;

        self.log(format!("add blob {:?} ({} bytes, comp: {})", path, orig_len, blob_len));

        let mut state = self.state.lock().unwrap();
        state.file_counter += 1;

        Ok(())
    }

    /// Mark backup as finished
    pub fn finish_backup(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        // test if all writer are correctly closed

        state.ensure_unfinished()?;

        state.finished = true;

        if state.dynamic_writers.len() != 0 {
            bail!("found open index writer - unable to finish backup");
        }

        if state.file_counter == 0 {
            bail!("backup does not contain valid files (file count == 0)");
        }

        Ok(())
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        self.worker.log(msg);
    }

    pub fn debug<S: AsRef<str>>(&self, msg: S) {
        if self.debug { self.worker.log(msg); }
    }

    pub fn format_response(&self, result: Result<Value, Error>) -> Response<Body> {
        match result {
            Ok(data) => (self.formatter.format_data)(data, self),
            Err(err) => (self.formatter.format_error)(err),
        }
    }

    /// Raise error if finished flag is not set
    pub fn ensure_finished(&self) -> Result<(), Error> {
        let state = self.state.lock().unwrap();
        if !state.finished {
            bail!("backup ended but finished flag is not set.");
        }
        Ok(())
    }

    /// Remove complete backup
    pub fn remove_backup(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        state.finished = true;

        self.datastore.remove_backup_dir(&self.backup_dir)?;

        Ok(())
    }
}

impl RpcEnvironment for BackupEnvironment {

    fn set_result_attrib(&mut self, name: &str, value: Value) {
        self.result_attributes.insert(name.into(), value);
    }

    fn get_result_attrib(&self, name: &str) -> Option<&Value> {
        self.result_attributes.get(name)
    }

    fn env_type(&self) -> RpcEnvironmentType {
        self.env_type
    }

    fn set_user(&mut self, _user: Option<String>) {
        panic!("unable to change user");
    }

    fn get_user(&self) -> Option<String> {
        Some(self.user.clone())
    }
}

impl AsRef<BackupEnvironment> for dyn RpcEnvironment {
    fn as_ref(&self) -> &BackupEnvironment {
        self.as_any().downcast_ref::<BackupEnvironment>().unwrap()
    }
}

impl AsRef<BackupEnvironment> for Box<dyn RpcEnvironment> {
    fn as_ref(&self) -> &BackupEnvironment {
        self.as_any().downcast_ref::<BackupEnvironment>().unwrap()
    }
}
