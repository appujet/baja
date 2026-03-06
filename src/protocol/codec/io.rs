use std::io::{self, Cursor, Read, Write};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

pub const V1: u8 = 1;
pub const V2: u8 = 2;
pub const V3: u8 = 3;

pub struct BinaryBuffer<T>(Cursor<T>);

impl BinaryBuffer<Vec<u8>> {
    pub fn new() -> Self {
        Self(Cursor::new(Vec::new()))
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(Cursor::new(Vec::with_capacity(capacity)))
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.0.into_inner()
    }
}

impl Default for BinaryBuffer<Vec<u8>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: AsRef<[u8]>> BinaryBuffer<T> {
    pub fn from_data(data: T) -> Self {
        Self(Cursor::new(data))
    }

    pub fn read_string(&mut self) -> io::Result<String> {
        let size = self.0.read_u16::<BigEndian>()? as usize;
        let mut content = vec![0u8; size];
        self.0.read_exact(&mut content)?;
        String::from_utf8(content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn read_nullable_string(&mut self) -> io::Result<Option<String>> {
        if self.0.read_u8()? != 0 {
            self.read_string().map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn read_json<V: serde::de::DeserializeOwned>(&mut self) -> io::Result<V> {
        let s = self.read_string()?;
        serde_json::from_str(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl BinaryBuffer<Vec<u8>> {
    pub fn write_string(&mut self, text: &str) -> io::Result<()> {
        let raw = text.as_bytes();
        self.0.write_u16::<BigEndian>(raw.len() as u16)?;
        self.0.write_all(raw)
    }

    pub fn write_nullable_string(&mut self, text: Option<&str>) -> io::Result<()> {
        match text {
            Some(s) => {
                self.0.write_u8(1)?;
                self.write_string(s)
            }
            None => self.0.write_u8(0),
        }
    }

    pub fn write_json<V: serde::Serialize>(&mut self, value: &V) -> io::Result<()> {
        let s = serde_json::to_string(value).map_err(io::Error::other)?;
        self.write_string(&s)
    }
}

impl<T> std::ops::Deref for BinaryBuffer<T> {
    type Target = Cursor<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for BinaryBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
