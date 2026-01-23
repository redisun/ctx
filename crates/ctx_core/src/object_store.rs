//! Content-addressed object storage with integrity verification.

use crate::error::{CtxError, Result};
use crate::object_id::{canonical_bytes, ObjectId, ObjectKind, MAGIC};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Maximum size for a single blob object (100 MB).
/// This prevents OOM attacks from maliciously large inputs.
const MAX_BLOB_SIZE: usize = 100 * 1024 * 1024;

/// Zstd compression level for object storage.
/// Level 3 provides a good balance between compression ratio and speed.
const COMPRESSION_LEVEL: i32 = 3;

/// Content-addressed object storage.
///
/// Objects are stored as zstd-compressed files with integrity verification.
/// The file path is derived from the object's BLAKE3 hash, enabling
/// deduplication and corruption detection.
///
/// # Examples
///
/// ```
/// use ctx_core::ObjectStore;
/// use tempfile::TempDir;
///
/// let tmp = TempDir::new().unwrap();
/// let store = ObjectStore::new(tmp.path().join("objects"));
///
/// // Store a blob
/// let id = store.put_blob(b"hello world").unwrap();
///
/// // Retrieve it
/// let data = store.get_blob(id).unwrap();
/// assert_eq!(data, b"hello world");
/// ```
pub struct ObjectStore {
    root: PathBuf,
}

impl ObjectStore {
    /// Creates a new ObjectStore at the given root directory.
    ///
    /// The directory will be created if it doesn't exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::ObjectStore;
    ///
    /// let store = ObjectStore::new("/tmp/objects");
    /// ```
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Returns the root directory of this object store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stores raw bytes and returns their content-addressed ID.
    ///
    /// If an object with the same content already exists, this is a no-op
    /// and returns the existing ID (deduplication).
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file writing fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::ObjectStore;
    /// use tempfile::TempDir;
    ///
    /// let tmp = TempDir::new().unwrap();
    /// let store = ObjectStore::new(tmp.path().join("objects"));
    ///
    /// let id = store.put_blob(b"content").unwrap();
    /// assert!(store.exists(id));
    /// ```
    pub fn put_blob(&self, data: &[u8]) -> Result<ObjectId> {
        // Check size limit to prevent OOM
        if data.len() > MAX_BLOB_SIZE {
            return Err(CtxError::BlobTooLarge {
                size: data.len(),
                limit: MAX_BLOB_SIZE,
            });
        }

        let id = ObjectId::hash_blob(data);

        // Check for existing object (deduplication)
        if self.exists(id) {
            return Ok(id);
        }

        let canonical = canonical_bytes(ObjectKind::Blob, data);
        self.write_object(id, &canonical)?;
        Ok(id)
    }

    /// Retrieves raw bytes by their content ID.
    ///
    /// # Errors
    ///
    /// Returns `ObjectNotFound` if the object doesn't exist.
    /// Returns `HashMismatch` if integrity verification fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::ObjectStore;
    /// use tempfile::TempDir;
    ///
    /// let tmp = TempDir::new().unwrap();
    /// let store = ObjectStore::new(tmp.path().join("objects"));
    ///
    /// let id = store.put_blob(b"test").unwrap();
    /// let data = store.get_blob(id).unwrap();
    /// assert_eq!(data, b"test");
    /// ```
    pub fn get_blob(&self, id: ObjectId) -> Result<Vec<u8>> {
        let (kind, payload) = self.read_object(id)?;

        if kind != ObjectKind::Blob {
            return Err(CtxError::CorruptedObject {
                path: self.object_path(id),
                reason: format!("expected Blob, got {:?}", kind),
            });
        }

        Ok(payload)
    }

    /// Stores a typed object using deterministic serialization.
    ///
    /// Uses postcard for compact, deterministic binary encoding.
    /// Same value always produces the same ID.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or storage fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::ObjectStore;
    /// use serde::{Serialize, Deserialize};
    /// use tempfile::TempDir;
    ///
    /// #[derive(Serialize, Deserialize, PartialEq, Debug)]
    /// struct Point { x: i32, y: i32 }
    ///
    /// let tmp = TempDir::new().unwrap();
    /// let store = ObjectStore::new(tmp.path().join("objects"));
    ///
    /// let point = Point { x: 10, y: 20 };
    /// let id = store.put_typed(&point).unwrap();
    ///
    /// let retrieved: Point = store.get_typed(id).unwrap();
    /// assert_eq!(retrieved, point);
    /// ```
    pub fn put_typed<T: Serialize>(&self, value: &T) -> Result<ObjectId> {
        let serialized =
            postcard::to_allocvec(value).map_err(|e| CtxError::Serialization(e.to_string()))?;

        let id = ObjectId::hash_typed(&serialized);

        // Check for existing object (deduplication)
        if self.exists(id) {
            return Ok(id);
        }

        let canonical = canonical_bytes(ObjectKind::Typed, &serialized);
        self.write_object(id, &canonical)?;
        Ok(id)
    }

    /// Retrieves and deserializes a typed object by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the object doesn't exist, is corrupted,
    /// or deserialization fails.
    pub fn get_typed<T: DeserializeOwned>(&self, id: ObjectId) -> Result<T> {
        let (kind, payload) = self.read_object(id)?;

        if kind != ObjectKind::Typed {
            return Err(CtxError::CorruptedObject {
                path: self.object_path(id),
                reason: format!("expected Typed, got {:?}", kind),
            });
        }

        postcard::from_bytes(&payload).map_err(|e| CtxError::Deserialization(e.to_string()))
    }

    /// Checks if an object exists in the store.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::{ObjectStore, ObjectId};
    /// use tempfile::TempDir;
    ///
    /// let tmp = TempDir::new().unwrap();
    /// let store = ObjectStore::new(tmp.path().join("objects"));
    ///
    /// let fake_id = ObjectId::from_bytes([0; 32]);
    /// assert!(!store.exists(fake_id));
    ///
    /// let id = store.put_blob(b"test").unwrap();
    /// assert!(store.exists(id));
    /// ```
    pub fn exists(&self, id: ObjectId) -> bool {
        self.object_path(id).exists()
    }

    /// Lists all objects in the store.
    ///
    /// Returns a vector of tuples containing:
    /// - ObjectId
    /// - Size in bytes (compressed on disk)
    /// - Last modification time
    ///
    /// This is used by garbage collection to find all objects.
    ///
    /// # Errors
    ///
    /// Returns an error if directory traversal fails or object IDs cannot be parsed.
    pub fn list_all_objects(&self) -> Result<Vec<(ObjectId, u64, std::time::SystemTime)>> {
        use std::time::SystemTime;

        let mut objects = Vec::new();

        // Check if objects directory exists
        if !self.root.exists() {
            return Ok(objects);
        }

        // Iterate through shard directories
        for shard_entry in fs::read_dir(&self.root)? {
            let shard_entry = shard_entry?;
            let shard_path = shard_entry.path();

            if !shard_path.is_dir() {
                continue;
            }

            // Iterate through objects in this shard
            for obj_entry in fs::read_dir(&shard_path)? {
                let obj_entry = obj_entry?;
                let obj_path = obj_entry.path();

                // Skip non-files and temp files
                if !obj_path.is_file() || obj_path.extension().is_some() {
                    continue;
                }

                // Parse object ID from filename
                let filename = obj_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| {
                        CtxError::GcError(format!("invalid object filename: {:?}", obj_path))
                    })?;

                let id = ObjectId::from_hex(filename).map_err(|e| {
                    CtxError::GcError(format!("failed to parse object ID {}: {}", filename, e))
                })?;

                // Get file metadata
                let metadata = fs::metadata(&obj_path)?;
                let size = metadata.len();
                let mtime = metadata.modified().unwrap_or_else(|_| SystemTime::now());

                objects.push((id, size, mtime));
            }
        }

        Ok(objects)
    }

    /// Deletes an object from the store.
    ///
    /// # Safety
    ///
    /// **DANGER**: This operation is irreversible and can corrupt the repository
    /// if used incorrectly. Only call this function after verifying that:
    ///
    /// 1. The object is unreachable from all refs (HEAD, branches, etc.)
    /// 2. No active sessions reference this object
    /// 3. The object is not part of an incomplete staging chain
    ///
    /// **Use the GC module instead** - it performs proper reachability analysis
    /// before deletion. Direct use of this function is almost always wrong.
    ///
    /// # Errors
    ///
    /// Returns an error if the object doesn't exist or deletion fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::ObjectStore;
    ///
    /// let mut store = ObjectStore::new("/tmp/objects");
    /// let id = store.put_blob(b"temporary").unwrap();
    ///
    /// // Later, during garbage collection:
    /// store.delete(id).unwrap();
    /// ```
    pub fn delete(&mut self, id: ObjectId) -> Result<()> {
        let path = self.object_path(id);

        if !path.exists() {
            return Err(CtxError::ObjectNotFound(id.as_hex()));
        }

        fs::remove_file(&path).map_err(|e| {
            CtxError::GcError(format!("failed to delete object {}: {}", id.as_hex(), e))
        })?;

        Ok(())
    }

    /// Computes the filesystem path for an object.
    fn object_path(&self, id: ObjectId) -> PathBuf {
        self.root.join(id.shard()).join(id.as_hex())
    }

    /// Writes compressed canonical bytes to disk atomically.
    fn write_object(&self, id: ObjectId, canonical: &[u8]) -> Result<()> {
        let path = self.object_path(id);
        let dir = path.parent().unwrap();

        // Ensure shard directory exists
        fs::create_dir_all(dir)?;

        // Compress with zstd
        let compressed = zstd::encode_all(canonical, COMPRESSION_LEVEL)
            .map_err(|e| CtxError::Compression(e.to_string()))?;

        // Atomic write: temp file + fsync + rename
        let tmp_path = path.with_extension("tmp");

        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(&compressed)?;
            file.sync_all()?;
        }

        fs::rename(&tmp_path, &path)?;

        // fsync parent directory (Unix-specific for crash safety)
        #[cfg(unix)]
        {
            if let Ok(dir_file) = File::open(dir) {
                let _ = dir_file.sync_all();
            }
        }

        Ok(())
    }

    /// Reads and verifies an object from disk.
    fn read_object(&self, id: ObjectId) -> Result<(ObjectKind, Vec<u8>)> {
        let path = self.object_path(id);

        if !path.exists() {
            return Err(CtxError::ObjectNotFound(id.as_hex()));
        }

        // Read compressed data
        let compressed = fs::read(&path)?;

        // Decompress
        let canonical = zstd::decode_all(compressed.as_slice())
            .map_err(|e| CtxError::Compression(e.to_string()))?;

        // Verify envelope format
        if canonical.len() < 14 {
            return Err(CtxError::CorruptedObject {
                path,
                reason: "object too small".to_string(),
            });
        }

        // Check magic bytes
        if &canonical[..5] != MAGIC {
            return Err(CtxError::CorruptedObject {
                path,
                reason: "invalid magic bytes".to_string(),
            });
        }

        // Parse kind
        let kind = match canonical[5] {
            1 => ObjectKind::Blob,
            2 => ObjectKind::Typed,
            k => {
                return Err(CtxError::CorruptedObject {
                    path,
                    reason: format!("unknown kind: {}", k),
                })
            }
        };

        // Parse and verify length
        let len = u64::from_le_bytes(canonical[6..14].try_into().unwrap()) as usize;
        let payload = &canonical[14..];

        if payload.len() != len {
            return Err(CtxError::CorruptedObject {
                path,
                reason: format!(
                    "length mismatch: header says {}, got {}",
                    len,
                    payload.len()
                ),
            });
        }

        // Verify hash
        let expected = match kind {
            ObjectKind::Blob => ObjectId::hash_blob(payload),
            ObjectKind::Typed => ObjectId::hash_typed(payload),
        };

        if expected != id {
            return Err(CtxError::HashMismatch {
                expected: id.as_hex(),
                actual: expected.as_hex(),
            });
        }

        Ok((kind, payload.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_blob_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let data = b"hello world";
        let id = store.put_blob(data).unwrap();
        let retrieved = store.get_blob(id).unwrap();

        assert_eq!(data.as_slice(), retrieved.as_slice());
    }

    #[test]
    fn test_content_addressing() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Same content = same ID
        let id1 = store.put_blob(b"test content").unwrap();
        let id2 = store.put_blob(b"test content").unwrap();
        assert_eq!(id1, id2);

        // Different content = different ID
        let id3 = store.put_blob(b"other content").unwrap();
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_deduplication() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let data = b"dedupe test";
        let id1 = store.put_blob(data).unwrap();

        // Verify object exists
        assert!(store.exists(id1));

        // Write same content again
        let id2 = store.put_blob(data).unwrap();
        assert_eq!(id1, id2);

        // Should still only have one copy
        assert!(store.exists(id1));
    }

    #[test]
    fn test_corruption_detection() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let id = store.put_blob(b"original content").unwrap();
        let path = tmp
            .path()
            .join("objects")
            .join(id.shard())
            .join(id.as_hex());

        // Corrupt the file
        std::fs::write(&path, b"corrupted data").unwrap();

        // Should detect corruption
        let result = store.get_blob(id);
        assert!(result.is_err());

        // Verify it's specifically a corruption/hash error
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                CtxError::CorruptedObject { .. }
                    | CtxError::HashMismatch { .. }
                    | CtxError::Compression(_)
            ),
            "Expected corruption-related error, got: {:?}",
            err
        );
    }

    #[test]
    fn test_typed_object_roundtrip() {
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct TestStruct {
            name: String,
            values: Vec<i32>,
        }

        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let obj = TestStruct {
            name: "test".to_string(),
            values: vec![1, 2, 3],
        };

        let id = store.put_typed(&obj).unwrap();
        let retrieved: TestStruct = store.get_typed(id).unwrap();

        assert_eq!(obj, retrieved);
    }

    #[test]
    fn test_typed_deterministic_serialization() {
        use serde::{Deserialize, Serialize};
        use std::collections::BTreeMap;

        #[derive(Serialize, Deserialize)]
        struct DeterministicStruct {
            // BTreeMap for deterministic order
            map: BTreeMap<String, i32>,
        }

        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let mut map = BTreeMap::new();
        map.insert("b".to_string(), 2);
        map.insert("a".to_string(), 1);
        map.insert("c".to_string(), 3);

        let obj = DeterministicStruct { map };

        // Same content should produce same ID across multiple calls
        let id1 = store.put_typed(&obj).unwrap();
        let id2 = store.put_typed(&obj).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_object_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let fake_id = ObjectId::from_bytes([0u8; 32]);
        let result = store.get_blob(fake_id);

        assert!(matches!(result, Err(CtxError::ObjectNotFound(_))));
    }

    #[test]
    fn test_exists() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let fake_id = ObjectId::from_bytes([0u8; 32]);
        assert!(!store.exists(fake_id));

        let id = store.put_blob(b"exists test").unwrap();
        assert!(store.exists(id));
    }

    #[test]
    fn test_empty_blob() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let id = store.put_blob(b"").unwrap();
        let retrieved = store.get_blob(id).unwrap();
        assert!(retrieved.is_empty());
    }

    #[test]
    fn test_large_blob() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // 1MB of data
        let data: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();
        let id = store.put_blob(&data).unwrap();
        let retrieved = store.get_blob(id).unwrap();

        assert_eq!(data, retrieved);
    }

    #[test]
    fn test_object_path_sharding() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        let id = ObjectId::from_bytes(bytes);

        let path = store.object_path(id);
        assert!(path.to_string_lossy().contains("/ab/"));
    }

    #[test]
    fn test_compression_reduces_size() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Highly compressible data
        let data = vec![b'a'; 10000];
        let id = store.put_blob(&data).unwrap();

        let path = store.object_path(id);
        let compressed_size = std::fs::metadata(path).unwrap().len();

        // Compressed should be much smaller than 10KB
        assert!(compressed_size < 1000);
    }

    #[test]
    fn test_wrong_kind_error() {
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize)]
        struct TestStruct {
            value: i32,
        }

        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Store as blob
        let id = store.put_blob(b"not a struct").unwrap();

        // Try to retrieve as typed - should fail
        let result: Result<TestStruct> = store.get_typed(id);
        assert!(result.is_err());
    }
}
