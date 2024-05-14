use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::time::SystemTime;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::base_storage::BaseStorage;
use crate::utils::read_version_2_fs;
use data_error::{ArklibError, Result};

/*
Note on `FileStorage` Versioning:

`FileStorage` is a basic key-value storage system that persists data to disk.

In version 2, `FileStorage` stored data in a plaintext format.
Starting from version 3, data is stored in JSON format.

For backward compatibility, we provide a helper function `read_version_2_fs` to read version 2 format.
*/
const STORAGE_VERSION: i32 = 3;

/// Represents a file storage system that persists data to disk.
pub struct FileStorage<K, V>
where
    K: Ord,
{
    label: String,
    path: PathBuf,
    timestamp: SystemTime,
    data: FileStorageData<K, V>,
}

/// A struct that represents the data stored in a [`FileStorage`] instance.
///
///
/// This is the data that is serialized and deserialized to and from disk.
#[derive(Serialize, Deserialize)]
pub struct FileStorageData<K, V>
where
    K: Ord,
{
    version: i32,
    entries: BTreeMap<K, V>,
}

impl<K, V> FileStorage<K, V>
where
    K: Ord + serde::Serialize + serde::de::DeserializeOwned + std::str::FromStr,
    V: serde::Serialize + serde::de::DeserializeOwned + std::str::FromStr,
{
    /// Create a new file storage with a diagnostic label and file path
    pub fn new(label: String, path: &Path) -> Self {
        let mut file_storage = Self {
            label,
            path: PathBuf::from(path),
            timestamp: SystemTime::now(),
            data: FileStorageData {
                version: STORAGE_VERSION,
                entries: BTreeMap::new(),
            },
        };

        // Load the data from the file
        file_storage.data.entries = match file_storage.read_fs() {
            Ok(data) => data,
            Err(_) => BTreeMap::new(),
        };
        file_storage
    }
}

impl<K, V> BaseStorage<K, V> for FileStorage<K, V>
where
    K: Ord + serde::Serialize + serde::de::DeserializeOwned + std::str::FromStr,
    V: serde::Serialize + serde::de::DeserializeOwned + std::str::FromStr,
{
    /// Set a key-value pair in the storage
    fn set(&mut self, key: K, value: V) {
        self.data.entries.insert(key, value);
        self.timestamp = std::time::SystemTime::now();
        self.write_fs()
            .expect("Failed to write data to disk");
    }

    /// Remove a key-value pair from the storage given a key
    fn remove(&mut self, id: &K) -> Result<()> {
        self.data.entries.remove(id).ok_or_else(|| {
            ArklibError::Storage(self.label.clone(), "Key not found".to_owned())
        })?;
        self.timestamp = std::time::SystemTime::now();
        self.write_fs()
            .expect("Failed to remove data from disk");
        Ok(())
    }

    /// Compare the timestamp of the storage file with the timestamp of the storage instance
    /// to determine if the storage file has been updated.
    fn is_storage_updated(&self) -> Result<bool> {
        let file_timestamp = fs::metadata(&self.path)?.modified()?;
        let file_time_secs = file_timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let self_time_secs = self
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Ok(file_time_secs > self_time_secs)
    }

    /// Read the data from the storage file
    fn read_fs(&mut self) -> Result<BTreeMap<K, V>> {
        if !self.path.exists() {
            return Err(ArklibError::Storage(
                self.label.clone(),
                "File does not exist".to_owned(),
            ));
        }

        // First check if the file starts with "version: 2"
        let file_content = std::fs::read_to_string(&self.path)?;
        if file_content.starts_with("version: 2") {
            // Attempt to parse the file using the legacy version 2 storage format of FileStorage.
            match read_version_2_fs(&self.path) {
                Ok(data) => {
                    log::info!(
                        "Version 2 storage format detected for {}",
                        self.label
                    );
                    self.timestamp = fs::metadata(&self.path)?.modified()?;
                    return Ok(data);
                }
                Err(_) => {
                    return Err(ArklibError::Storage(
                        self.label.clone(),
                        "Storage seems to be version 2, but failed to parse"
                            .to_owned(),
                    ));
                }
            };
        }

        let file = fs::File::open(&self.path)?;
        let data: FileStorageData<K, V> = serde_json::from_reader(file)
            .map_err(|err| {
                ArklibError::Storage(self.label.clone(), err.to_string())
            })?;
        let version = data.version;
        if version != STORAGE_VERSION {
            return Err(ArklibError::Storage(
                self.label.clone(),
                format!(
                    "Storage version mismatch: expected {}, got {}",
                    STORAGE_VERSION, version
                ),
            ));
        }
        self.timestamp = fs::metadata(&self.path)?.modified()?;

        Ok(data.entries)
    }

    /// Write the data to the storage file
    fn write_fs(&mut self) -> Result<()> {
        let parent_dir = self.path.parent().ok_or_else(|| {
            ArklibError::Storage(
                self.label.clone(),
                "Failed to get parent directory".to_owned(),
            )
        })?;
        fs::create_dir_all(parent_dir)?;
        let file = File::create(&self.path)?;
        let mut writer = BufWriter::new(file);
        let value_data = serde_json::to_string_pretty(&self.data)?;
        writer.write_all(value_data.as_bytes())?;

        let new_timestamp = fs::metadata(&self.path)?.modified()?;
        if new_timestamp == self.timestamp {
            return Err("Timestamp has not been updated".into());
        }
        self.timestamp = new_timestamp;

        log::info!(
            "{} {} entries have been written",
            self.label,
            self.data.entries.len()
        );
        Ok(())
    }

    /// Erase the storage file from disk
    fn erase(&self) -> Result<()> {
        fs::remove_file(&self.path).map_err(|err| {
            ArklibError::Storage(self.label.clone(), err.to_string())
        })
    }
}

impl<K, V> AsRef<BTreeMap<K, V>> for FileStorage<K, V>
where
    K: Ord,
{
    fn as_ref(&self) -> &BTreeMap<K, V> {
        &self.data.entries
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use tempdir::TempDir;

    use crate::{base_storage::BaseStorage, file_storage::FileStorage};

    #[test]
    fn test_file_storage_write_read() {
        let temp_dir =
            TempDir::new("tmp").expect("Failed to create temporary directory");
        let storage_path = temp_dir.path().join("test_storage.txt");

        let mut file_storage =
            FileStorage::new("TestStorage".to_string(), &storage_path);

        file_storage.set("key1".to_string(), "value1".to_string());
        file_storage.set("key2".to_string(), "value2".to_string());

        assert!(file_storage.remove(&"key1".to_string()).is_ok());
        let data_read: BTreeMap<_, _> = file_storage
            .read_fs()
            .expect("Failed to read data from disk");

        assert_eq!(data_read.len(), 1);
        assert_eq!(data_read.get("key2").map(|v| v.as_str()), Some("value2"))
    }

    #[test]
    fn test_file_storage_auto_delete() {
        let temp_dir =
            TempDir::new("tmp").expect("Failed to create temporary directory");
        let storage_path = temp_dir.path().join("test_storage.txt");

        let mut file_storage =
            FileStorage::new("TestStorage".to_string(), &storage_path);

        file_storage.set("key1".to_string(), "value1".to_string());
        file_storage.set("key1".to_string(), "value2".to_string());

        assert_eq!(storage_path.exists(), true);

        if let Err(err) = file_storage.erase() {
            panic!("Failed to delete file: {:?}", err);
        }
        assert_eq!(storage_path.exists(), false);
    }

    #[test]
    fn test_file_storage_is_storage_updated() {
        let temp_dir =
            TempDir::new("tmp").expect("Failed to create temporary directory");
        let storage_path = temp_dir.path().join("teststorage.txt");

        let mut file_storage =
            FileStorage::new("TestStorage".to_string(), &storage_path);

        file_storage.set("key1".to_string(), "value1".to_string());
        assert_eq!(file_storage.is_storage_updated().unwrap(), false);

        std::thread::sleep(std::time::Duration::from_secs(1));

        // External data manipulation
        let mut mirror_storage =
            FileStorage::new("TestStorage".to_string(), &storage_path);

        mirror_storage.set("key1".to_string(), "value3".to_string());
        assert_eq!(mirror_storage.is_storage_updated().unwrap(), false);

        assert_eq!(file_storage.is_storage_updated().unwrap(), true);
    }
}

use jni::signature::{JavaType, ReturnType};
// This is the interface to the JVM that we'll call the majority of our
// methods on.
use jni::JNIEnv;

// These objects are what you should use as arguments to your native
// function. They carry extra lifetime information to prevent them escaping
// this context and getting used after being GC'd.
use jni::objects::{JClass, JObject, JString, JValue};

// This is just a pointer. We'll be returning it from our function. We
// can't return one of the objects with lifetime information because the
// lifetime checker won't let us.
use jni::sys::{jboolean, jlong, jobject, jvalue};

impl FileStorage<String, String> {
    fn from_jlong<'a>(value: jlong) -> &'a mut Self {
        unsafe { &mut *(value as *mut FileStorage<String, String>) }
    }
}

#[no_mangle]
pub extern "system" fn Java_FileStorage_create(
    env: &mut JNIEnv,
    _class: JClass,
    label: JString,
    path: JString,
) -> jlong {
    let label: String = env.get_string(&label).unwrap().into();
    let path: String = env.get_string(&path).unwrap().into();

    let file_storage: FileStorage<String, String> =
        FileStorage::new(label, Path::new(&path));
    Box::into_raw(Box::new(file_storage)) as jlong
}

#[no_mangle]
pub extern "system" fn Java_FileStorage_set(
    env: &mut JNIEnv,
    _class: JClass,
    id: JString,
    value: JString,
    file_storage_ptr: jlong,
) {
    let id: String = env.get_string(&id).unwrap().into();
    let value: String = env.get_string(&value).unwrap().into();

    FileStorage::from_jlong(file_storage_ptr).set(id, value);
}

#[no_mangle]
pub extern "system" fn Java_FileStorage_remove(
    env: &mut JNIEnv,
    _class: JClass,
    id: JString,
    file_storage_ptr: jlong,
) {
    let id: String = env.get_string(&id).unwrap().into();

    FileStorage::from_jlong(file_storage_ptr)
        .remove(&id)
        .unwrap()
}

#[no_mangle]
pub extern "system" fn Java_FileStorage_is_storage_updated(
    _env: &mut JNIEnv,
    _class: JClass,
    file_storage_ptr: jlong,
) -> jboolean {
    match FileStorage::from_jlong(file_storage_ptr).is_storage_updated() {
        Ok(updated) => updated as jboolean,
        Err(_) => 0, // handle error here
    }
}

#[no_mangle]
pub extern "system" fn Java_FileStorage_read_fs(
    env: &mut JNIEnv,
    _class: JClass,
    file_storage_ptr: jlong,
) -> jobject {
    let data: BTreeMap<String, String> =
        FileStorage::from_jlong(file_storage_ptr)
            .read_fs()
            .unwrap();

    // Create a new LinkedHashMap object
    let linked_hash_map_class =
        env.find_class("java/util/LinkedHashMap").unwrap();
    let linked_hash_map = env
        .new_object(linked_hash_map_class, "()V", &[])
        .unwrap();

    // Get the put method ID
    let put_method_id = env
        .get_method_id(
            "java/util/LinkedHashMap",
            "put",
            "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
        )
        .unwrap();

    // Insert each key-value pair from the BTreeMap into the LinkedHashMap
    for (key, value) in data {
        let j_key = env.new_string(key).unwrap();
        let j_value = env.new_string(value).unwrap();
        let j_key = JValue::from(&j_key).as_jni();
        let j_value = JValue::from(&j_value).as_jni();
        unsafe {
            env.call_method_unchecked(
                &linked_hash_map,
                put_method_id,
                ReturnType::Object,
                &[j_key, j_value],
            )
            .unwrap()
        };
    }

    // Return the LinkedHashMap as a raw pointer
    linked_hash_map.as_raw()
}

#[no_mangle]
pub extern "system" fn Java_FileStorage_write_fs(
    _env: &mut JNIEnv,
    _class: JClass,
    file_storage_ptr: jlong,
) {
    FileStorage::from_jlong(file_storage_ptr)
        .write_fs()
        .unwrap()
}

///! Safety: The FileStorage instance is dropped after this call
#[no_mangle]
pub extern "system" fn Java_FileStorage_erase(
    _env: &mut JNIEnv,
    _class: JClass,
    file_storage_ptr: jlong,
) {
    let file_storage = unsafe {
        Box::from_raw(file_storage_ptr as *mut FileStorage<String, String>)
    };
    file_storage.erase().unwrap();
}
