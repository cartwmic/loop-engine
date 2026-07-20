use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use loop_engine_core::model::bounded::IDENTIFIER_UTF8_BYTES;
use serde_json::json;

use super::error::TraceError;
use super::event::{TraceCategory, TraceEvent};
use super::rotation::{
    EvictedTrace, RotationFiles, TRACE_FILE_MAX_BYTES, TRACE_PROVIDER_CALL_RESERVATION_BYTES,
    ensure_additional_capacity, initialize, with_rotation, write_reservation,
};

pub struct TraceWriter {
    directory: PathBuf,
    path: PathBuf,
    request_id: String,
    trace: File,
    rotation: RotationFiles,
    unused_reservation: u64,
    provider_remainder: Option<u64>,
    pending_evictions: Vec<EvictedTrace>,
    encoded_bytes: u64,
    failed: bool,
    closed: bool,
}

impl TraceWriter {
    pub fn create(directory: &Path, request_id: impl Into<String>) -> Result<Self, TraceError> {
        let request_id = request_id.into();
        if request_id.is_empty()
            || request_id.len() > IDENTIFIER_UTF8_BYTES
            || request_id.as_bytes().contains(&b'/')
            || request_id.as_bytes().contains(&0)
        {
            return Err(TraceError::Collision(directory.join("invalid-request-id")));
        }
        let mut rotation = initialize(directory, &request_id)?;
        let pending_evictions = std::mem::take(&mut rotation.evicted);
        let path = directory.join(format!("{request_id}.jsonl"));
        let trace = match OpenOptions::new()
            .append(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                let _ = std::fs::remove_file(&rotation.sidecar_path);
                return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
                    TraceError::Collision(path)
                } else {
                    TraceError::io(path, error)
                });
            }
        };
        let writer = Self {
            directory: directory.to_owned(),
            path,
            request_id,
            trace,
            rotation,
            unused_reservation: super::rotation::TRACE_INIT_RESERVATION_BYTES,
            provider_remainder: None,
            pending_evictions,
            encoded_bytes: 0,
            failed: false,
            closed: false,
        };
        Ok(writer)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn write(&mut self, event: &TraceEvent) -> Result<usize, TraceError> {
        if self.failed {
            return Err(TraceError::SinkFailed);
        }
        if event.request_id != self.request_id {
            return Err(TraceError::ReservationExhausted);
        }
        if let Some(field) = event.payload.keys().find(|field| {
            matches!(
                field.as_str(),
                "trace_schema_version" | "ts" | "request_id" | "category" | "event"
            )
        }) {
            return Err(TraceError::ReservedPayloadField(field.clone()));
        }
        let first_line = self.encoded_bytes == 0;
        let written = self.write_line(event)?;
        if first_line {
            let evicted = std::mem::take(&mut self.pending_evictions);
            self.write_rotation_events(evicted)?;
        }
        Ok(written)
    }

    fn write_line(&mut self, event: &TraceEvent) -> Result<usize, TraceError> {
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        let length = u64::try_from(bytes.len()).expect("encoded event length fits u64");
        if length > self.unused_reservation
            || self
                .provider_remainder
                .is_some_and(|remainder| length > remainder)
        {
            return Err(TraceError::ReservationExhausted);
        }
        if self.encoded_bytes.saturating_add(length) > TRACE_FILE_MAX_BYTES {
            return Err(TraceError::FileLimit {
                max: TRACE_FILE_MAX_BYTES,
            });
        }
        let directory = self.directory.clone();
        let path = self.path.clone();
        let sidecar_path = self.rotation.sidecar_path.clone();
        let prior_encoded = self.encoded_bytes;
        let prior_unused = self.unused_reservation;
        let new_unused = prior_unused - length;
        let mut reconciled = None;
        let result = with_rotation(&directory, &self.rotation.lock, || {
            let trace_result = self
                .trace
                .write_all(&bytes)
                .and_then(|()| self.trace.flush())
                .and_then(|()| self.trace.sync_data());
            if let Err(error) = trace_result {
                let observed = self
                    .trace
                    .metadata()
                    .map(|metadata| metadata.len())
                    .unwrap_or(prior_encoded);
                let observed_delta = observed.saturating_sub(prior_encoded).min(prior_unused);
                let observed_unused = prior_unused - observed_delta;
                let _ =
                    write_reservation(&mut self.rotation.sidecar, &sidecar_path, observed_unused);
                reconciled = Some((observed, observed_unused, observed_delta));
                return Err(TraceError::io(&path, error));
            }
            if let Err(error) =
                write_reservation(&mut self.rotation.sidecar, &sidecar_path, new_unused)
            {
                let observed = self
                    .trace
                    .metadata()
                    .map(|metadata| metadata.len())
                    .unwrap_or(prior_encoded.saturating_add(length));
                let observed_delta = observed.saturating_sub(prior_encoded).min(prior_unused);
                reconciled = Some((observed, prior_unused - observed_delta, observed_delta));
                return Err(error);
            }
            Ok(())
        });
        if let Err(error) = result {
            self.failed = true;
            if let Some((observed, observed_unused, observed_delta)) = reconciled {
                self.encoded_bytes = observed;
                self.unused_reservation = observed_unused;
                if let Some(remainder) = &mut self.provider_remainder {
                    *remainder = remainder.saturating_sub(observed_delta);
                }
            }
            return Err(error);
        }
        self.unused_reservation = new_unused;
        self.encoded_bytes = prior_encoded + length;
        if let Some(remainder) = &mut self.provider_remainder {
            *remainder = remainder.saturating_sub(length);
        }
        Ok(bytes.len())
    }

    pub fn reserve_provider_call(&mut self) -> Result<(), TraceError> {
        if self.failed {
            return Err(TraceError::SinkFailed);
        }
        if self.provider_remainder.is_some() {
            return Err(TraceError::ReservationExhausted);
        }
        let directory = self.directory.clone();
        let sidecar_path = self.rotation.sidecar_path.clone();
        let new_unused = self
            .unused_reservation
            .saturating_add(TRACE_PROVIDER_CALL_RESERVATION_BYTES);
        let mut sidecar_attempted = false;
        let evicted = match with_rotation(&directory, &self.rotation.lock, || {
            let evicted =
                ensure_additional_capacity(&directory, TRACE_PROVIDER_CALL_RESERVATION_BYTES)?;
            sidecar_attempted = true;
            write_reservation(&mut self.rotation.sidecar, &sidecar_path, new_unused)?;
            Ok(evicted)
        }) {
            Ok(evicted) => evicted,
            Err(error) if !sidecar_attempted => return Err(error),
            Err(error) => {
                self.unused_reservation = new_unused;
                self.provider_remainder = Some(TRACE_PROVIDER_CALL_RESERVATION_BYTES);
                self.failed = true;
                return Err(error);
            }
        };
        self.unused_reservation = new_unused;
        if self.encoded_bytes == 0 {
            self.pending_evictions.extend(evicted);
        } else if let Err(error) = self.write_rotation_events(evicted) {
            let rollback = self
                .unused_reservation
                .saturating_sub(TRACE_PROVIDER_CALL_RESERVATION_BYTES);
            with_rotation(&directory, &self.rotation.lock, || {
                write_reservation(&mut self.rotation.sidecar, &sidecar_path, rollback)
            })?;
            self.unused_reservation = rollback;
            return Err(error);
        }
        self.provider_remainder = Some(TRACE_PROVIDER_CALL_RESERVATION_BYTES);
        Ok(())
    }

    pub fn release_provider_call(&mut self) -> Result<u64, TraceError> {
        let remainder = self
            .provider_remainder
            .ok_or(TraceError::NoProviderReservation)?;
        let new_unused = self.unused_reservation.saturating_sub(remainder);
        let directory = self.directory.clone();
        let sidecar_path = self.rotation.sidecar_path.clone();
        let result = with_rotation(&directory, &self.rotation.lock, || {
            write_reservation(&mut self.rotation.sidecar, &sidecar_path, new_unused)
        });
        self.provider_remainder = None;
        self.unused_reservation = new_unused;
        if let Err(error) = result {
            self.failed = true;
            return Err(error);
        }
        Ok(remainder)
    }

    pub fn close(mut self) -> Result<(), TraceError> {
        if self.provider_remainder.is_some() {
            self.release_provider_call()?;
        }
        let directory = self.directory.clone();
        let sidecar_path = self.rotation.sidecar_path.clone();
        with_rotation(&directory, &self.rotation.lock, || {
            self.trace
                .flush()
                .and_then(|()| self.trace.sync_data())
                .map_err(|error| TraceError::io(&self.path, error))?;
            std::fs::remove_file(&sidecar_path)
                .map_err(|error| TraceError::io(&sidecar_path, error))
        })?;
        self.closed = true;
        Ok(())
    }

    pub fn unused_reservation(&self) -> u64 {
        self.unused_reservation
    }

    fn write_rotation_events(&mut self, evicted: Vec<EvictedTrace>) -> Result<(), TraceError> {
        let provider_remainder = self.provider_remainder.take();
        for victim in evicted {
            let mut payload = BTreeMap::new();
            payload.insert("evicted_path".into(), json!(victim.path.to_string_lossy()));
            payload.insert(
                "encoded_bytes_reclaimed".into(),
                json!(victim.encoded_bytes),
            );
            let event = TraceEvent::new(
                self.request_id(),
                TraceCategory::Trace,
                "rotation_evict",
                payload,
            );
            if let Err(error) = self.write(&event) {
                self.provider_remainder = provider_remainder;
                return Err(error);
            }
        }
        self.provider_remainder = provider_remainder;
        Ok(())
    }
}

impl Drop for TraceWriter {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.trace.flush();
        }
    }
}
