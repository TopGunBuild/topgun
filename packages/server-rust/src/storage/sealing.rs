//! Encryption at rest of record values and WAL frames: the keyring, the
//! value envelope, the segment key and the master-key provider
//! (`TG-ENC-001`, `TG-ENC-002`, `TG-ENC-003`, `TG-ENC-005`).
//!
//! A record value is sealed with XChaCha20-Poly1305 under a data key (DEK)
//! named by its key id; every DEK is kept on disk only wrapped under the
//! master key (KEK) of a [`MasterKeyProvider`], in the store's `keyring`
//! metadata row.
//!
//! # Value envelope (`TG-ENC-002`)
//!
//! `0xC1 ‖ env_version u8 (1) ‖ cipher u8 (1) ‖ key_id u32 BE ‖ nonce [24] ‖
//! ciphertext ‖ tag [16]` — 47 bytes of overhead. `MessagePack` never uses
//! `0xC1`, so a sealed value is told from a plain one by its first byte. The
//! AAD is the row's full engine key followed by the first 7 envelope bytes,
//! so a sealed value opens only at the row it was sealed for, under the key
//! id its header names.
//!
//! # Sealed WAL frames (`TG-ENC-003`, `TG-ENC-005`)
//!
//! A sealed segment's frames are sealed under its [`SegmentKey`], derived
//! from the DEK its header names, its random salt and its other header bytes.
//! The nonce is the frame's index in the segment, never stored; it is safe
//! because a process appends only to segments it created, each with a fresh
//! salt, and takes every index once under the append lock.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{AeadInPlace, KeyInit, OsRng};
use chacha20poly1305::{Key, Tag, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// First byte of a sealed value; a byte `MessagePack` never uses.
pub const SEALED_MARK: u8 = 0xC1;
/// The one envelope version this binary reads and writes.
pub const ENVELOPE_VERSION: u8 = 1;
/// Cipher byte of XChaCha20-Poly1305.
pub const CIPHER_XCHACHA20_POLY1305: u8 = 1;
/// Mark, version, cipher and key id: the envelope bytes the AAD covers.
const HEADER_LEN: usize = 7;
/// A random nonce per seal.
pub const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// Bytes a sealed value adds to its plaintext.
pub const ENVELOPE_OVERHEAD: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

/// The one keyring row version this binary reads and writes.
pub const KEYRING_VERSION: u32 = 1;
const DEK_WRAP_AAD: &[u8] = b"topgun/dek-wrap/1";
const KEK_FINGERPRINT_DOMAIN: &[u8] = b"topgun/kek-fp/1";

/// The bytes of a DEK or a KEK.
pub const KEY_LEN: usize = 32;

/// The first 8 bytes of SHA-256 over `"topgun/kek-fp/1" ‖ KEK`.
pub type Fingerprint = [u8; 8];

/// A plaintext key, zeroed on drop.
pub type SecretKey = Zeroizing<[u8; KEY_LEN]>;

/// The fingerprint of `kek`.
#[must_use]
pub fn kek_fingerprint(kek: &[u8; KEY_LEN]) -> Fingerprint {
    let digest = Sha256::new()
        .chain_update(KEK_FINGERPRINT_DOMAIN)
        .chain_update(kek)
        .finalize();
    let mut fingerprint = [0u8; 8];
    fingerprint.copy_from_slice(&digest[..8]);
    fingerprint
}

/// `fingerprint` as 16 lowercase hex characters.
#[must_use]
pub fn fingerprint_hex(fingerprint: &Fingerprint) -> String {
    hex::encode(fingerprint)
}

fn random_bytes<const N: usize>() -> anyhow::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| anyhow::anyhow!("the OS random source failed: {e}"))?;
    Ok(bytes)
}

/// A fresh DEK from the OS CSPRNG.
///
/// # Errors
///
/// When the OS random source fails.
pub fn generate_key() -> anyhow::Result<SecretKey> {
    Ok(Zeroizing::new(random_bytes::<KEY_LEN>()?))
}

fn cipher(key: &[u8; KEY_LEN]) -> XChaCha20Poly1305 {
    // The cipher zeroes its copy of the key on drop.
    XChaCha20Poly1305::new(Key::from_slice(key))
}

/// The state of a key in the keyring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    /// New values are sealed under it; at most one key is active.
    Active,
    /// Opens values sealed under it, seals nothing.
    DecryptOnly,
}

/// One DEK of the keyring row, wrapped under the KEK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    /// Key id, never 0 and never reused.
    pub id: u32,
    /// Active or decrypt-only.
    pub state: KeyState,
    /// The wrap's random nonce.
    pub nonce: [u8; NONCE_LEN],
    /// The DEK sealed under the KEK, with its tag.
    pub wrapped: Vec<u8>,
}

/// The `keyring` metadata row (`TG-ENC-001`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyringRow {
    /// [`KEYRING_VERSION`].
    pub version: u32,
    /// The fingerprint of the KEK every key is wrapped under.
    pub kek_fingerprint: Fingerprint,
    /// The id the next key gets; above every id in `keys`.
    pub next_key_id: u32,
    /// The keys, in the order they were added.
    pub keys: Vec<WrappedKey>,
}

impl KeyringRow {
    /// The row's bytes.
    ///
    /// # Errors
    ///
    /// The encoder's error.
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        Ok(rmp_serde::to_vec_named(self)?)
    }

    /// Decodes a row and checks it: the version is known, no id is 0 or
    /// repeated, at most one key is active, and `next_key_id` is above every
    /// id. The error says which rule the row breaks.
    ///
    /// # Errors
    ///
    /// When the bytes are not such a row.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let row: KeyringRow = rmp_serde::from_slice(bytes)
            .map_err(|e| format!("the keyring row cannot be decoded: {e}"))?;
        if row.version != KEYRING_VERSION {
            return Err(format!(
                "the keyring is version {}, not {KEYRING_VERSION}",
                row.version
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for key in &row.keys {
            if key.id == 0 {
                return Err("the keyring holds key id 0, which is never issued".to_string());
            }
            if !ids.insert(key.id) {
                return Err(format!("the keyring holds key id {} twice", key.id));
            }
        }
        let active = row
            .keys
            .iter()
            .filter(|k| k.state == KeyState::Active)
            .count();
        if active > 1 {
            return Err(format!("the keyring holds {active} active keys"));
        }
        let largest = ids.last().copied().unwrap_or(0);
        if row.next_key_id <= largest {
            return Err(format!(
                "the keyring's next key id {} is not above its largest id {largest}",
                row.next_key_id
            ));
        }
        Ok(row)
    }

    /// The active key's id.
    #[must_use]
    pub fn active_id(&self) -> Option<u32> {
        self.keys
            .iter()
            .find(|k| k.state == KeyState::Active)
            .map(|k| k.id)
    }
}

/// A master-key provider: wraps and unwraps DEKs under a KEK it never hands
/// out. The keyring format does not depend on the provider.
pub trait MasterKeyProvider: Send + Sync {
    /// The provider's name in log lines (`file`).
    fn name(&self) -> &'static str;

    /// The fingerprint of the KEK.
    fn fingerprint(&self) -> Fingerprint;

    /// Wraps the DEK `dek` of key id `id`: `(nonce, wrapped)`.
    ///
    /// # Errors
    ///
    /// When the provider cannot wrap.
    fn wrap(&self, id: u32, dek: &[u8; KEY_LEN]) -> anyhow::Result<([u8; NONCE_LEN], Vec<u8>)>;

    /// Unwraps the DEK of key id `id`.
    ///
    /// # Errors
    ///
    /// When the wrap does not authenticate under this KEK and id.
    fn unwrap(&self, id: u32, nonce: &[u8; NONCE_LEN], wrapped: &[u8])
        -> anyhow::Result<SecretKey>;
}

fn wrap_aad(id: u32) -> Vec<u8> {
    let mut aad = Vec::with_capacity(DEK_WRAP_AAD.len() + 4);
    aad.extend_from_slice(DEK_WRAP_AAD);
    aad.extend_from_slice(&id.to_be_bytes());
    aad
}

/// Why a master key file was refused. No variant carries key material.
#[derive(Debug, thiserror::Error)]
pub enum KeyFileError {
    /// The file cannot be read, or its metadata cannot.
    #[error("the master key file {path} cannot be read: {source}")]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        source: std::io::Error,
    },
    /// Group or others may access the file.
    #[error(
        "the master key file {path} is accessible by group or others (mode {mode:o}); allow its owner only (chmod 600)"
    )]
    TooPermissive {
        /// The file.
        path: PathBuf,
        /// The permission bits of the file (of a symlink's target).
        mode: u32,
    },
    /// Not 64 hex characters with an optional trailing newline.
    #[error("the master key file {path} does not hold exactly 64 hexadecimal characters")]
    Malformed {
        /// The file.
        path: PathBuf,
    },
}

/// The `file` master-key provider: a 256-bit KEK read from a file of 64 hex
/// characters and an optional trailing newline.
pub struct FileKeyProvider {
    kek: SecretKey,
    fingerprint: Fingerprint,
}

impl fmt::Debug for FileKeyProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileKeyProvider")
            .field("fingerprint", &fingerprint_hex(&self.fingerprint))
            .finish_non_exhaustive()
    }
}

impl FileKeyProvider {
    /// Reads the KEK from `path`. A file group or others may access is
    /// refused, judged on the target of a symlink.
    ///
    /// # Errors
    ///
    /// A [`KeyFileError`].
    pub fn load(path: &Path) -> Result<Self, KeyFileError> {
        let unreadable = |source| KeyFileError::Unreadable {
            path: path.to_path_buf(),
            source,
        };
        // One open: the mode checked and the bytes read are those of the same
        // file, even if the path is swapped in between.
        let mut file = std::fs::File::open(path).map_err(unreadable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = file.metadata().map_err(unreadable)?.permissions().mode() & 0o7777;
            if mode & 0o077 != 0 {
                return Err(KeyFileError::TooPermissive {
                    path: path.to_path_buf(),
                    mode,
                });
            }
        }
        let mut content = Zeroizing::new(Vec::new());
        std::io::Read::read_to_end(&mut file, &mut content).map_err(unreadable)?;
        let hex_part = content.strip_suffix(b"\n").unwrap_or(&content);
        let mut kek = Zeroizing::new([0u8; KEY_LEN]);
        // The decoder's own error names the offending character, which is
        // key material, so it is dropped.
        if hex_part.len() != 2 * KEY_LEN || hex::decode_to_slice(hex_part, &mut kek[..]).is_err() {
            return Err(KeyFileError::Malformed {
                path: path.to_path_buf(),
            });
        }
        Ok(Self::from_kek(kek))
    }

    /// A provider over `kek`.
    pub(crate) fn from_kek(kek: SecretKey) -> Self {
        let fingerprint = kek_fingerprint(&kek);
        Self { kek, fingerprint }
    }
}

impl MasterKeyProvider for FileKeyProvider {
    fn name(&self) -> &'static str {
        "file"
    }

    fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    fn wrap(&self, id: u32, dek: &[u8; KEY_LEN]) -> anyhow::Result<([u8; NONCE_LEN], Vec<u8>)> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let mut wrapped = dek.to_vec();
        let tag = cipher(&self.kek)
            .encrypt_in_place_detached(XNonce::from_slice(&nonce), &wrap_aad(id), &mut wrapped)
            .map_err(|_| anyhow::anyhow!("wrapping key {id} failed"))?;
        wrapped.extend_from_slice(&tag);
        Ok((nonce, wrapped))
    }

    fn unwrap(
        &self,
        id: u32,
        nonce: &[u8; NONCE_LEN],
        wrapped: &[u8],
    ) -> anyhow::Result<SecretKey> {
        let failed = || anyhow::anyhow!("the wrap of key {id} does not authenticate");
        if wrapped.len() != KEY_LEN + TAG_LEN {
            return Err(failed());
        }
        let mut dek = Zeroizing::new([0u8; KEY_LEN]);
        dek.copy_from_slice(&wrapped[..KEY_LEN]);
        cipher(&self.kek)
            .decrypt_in_place_detached(
                XNonce::from_slice(nonce),
                &wrap_aad(id),
                &mut dek[..],
                Tag::from_slice(&wrapped[KEY_LEN..]),
            )
            .map_err(|_| failed())?;
        Ok(dek)
    }
}

/// Encryption at rest on or off (`TOPGUN_ENCRYPTION_AT_REST`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionMode {
    /// New values are stored plain; a sealed store is unsealed.
    Off,
    /// New values are sealed; a plain store is sealed.
    On,
}

/// What a store open is given to seal, unseal and unwrap with.
#[derive(Clone)]
pub struct Encryption {
    /// On or off.
    pub mode: EncryptionMode,
    /// The configured master key; required under `On` and for a store that
    /// holds a keyring.
    pub provider: Option<Arc<dyn MasterKeyProvider>>,
    /// The master key a rotation re-wraps from.
    pub previous: Option<Arc<dyn MasterKeyProvider>>,
}

impl Encryption {
    /// Off, with no provider.
    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: EncryptionMode::Off,
            provider: None,
            previous: None,
        }
    }
}

/// Which way a seal pass rewrites values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassDirection {
    /// Every value to sealed under the active key.
    Seal,
    /// Every value to plain.
    Unseal,
}

impl PassDirection {
    fn byte(self) -> u8 {
        match self {
            Self::Seal => 0x01,
            Self::Unseal => 0x02,
        }
    }

    /// `seal` or `unseal`, for log lines.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Seal => "seal",
            Self::Unseal => "unseal",
        }
    }
}

/// The `seal_pass` metadata row: `direction u8 ‖ cursor engine-key bytes`
/// (`TG-ENC-004`). Rows at or before the cursor are in the target form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealPass {
    /// Seal or unseal.
    pub direction: PassDirection,
    /// The engine key of the last row done; empty before the first batch.
    pub cursor: Vec<u8>,
}

impl SealPass {
    /// The row's bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1 + self.cursor.len());
        bytes.push(self.direction.byte());
        bytes.extend_from_slice(&self.cursor);
        bytes
    }

    /// Decodes a row.
    ///
    /// # Errors
    ///
    /// An empty row or an unknown direction byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let direction = match bytes.first() {
            Some(0x01) => PassDirection::Seal,
            Some(0x02) => PassDirection::Unseal,
            Some(other) => return Err(format!("the seal pass direction {other:#04x} is unknown")),
            None => return Err("the seal pass row is empty".to_string()),
        };
        Ok(Self {
            direction,
            cursor: bytes[1..].to_vec(),
        })
    }
}

/// Why a sealed value was not opened (`TG-ENC-002`). No variant carries the
/// record key or any plaintext.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueOpenError {
    /// Shorter than the envelope.
    #[error("sealed value is {len} bytes, shorter than the {ENVELOPE_OVERHEAD}-byte envelope")]
    Truncated {
        /// Its length.
        len: usize,
    },
    /// An envelope version this binary does not know.
    #[error("sealed value has envelope version {0}, not {ENVELOPE_VERSION}")]
    UnknownVersion(u8),
    /// A cipher this binary does not know.
    #[error("sealed value has cipher {0}, not {CIPHER_XCHACHA20_POLY1305}")]
    UnknownCipher(u8),
    /// The key id is not in the keyring.
    #[error("sealed value names key id {0}, which is not in the keyring")]
    UnknownKeyId(u32),
    /// The tag does not match: a changed byte, or a value moved to another row.
    #[error("value authentication failed for map {map_id} partition {partition}")]
    AuthenticationFailed {
        /// The map id of the row read.
        map_id: u32,
        /// The partition of the row read.
        partition: u32,
    },
}

/// The key id a sealed value names, `None` for a plain value or one too
/// short to carry a header.
#[must_use]
pub fn sealed_key_id(value: &[u8]) -> Option<u32> {
    match value {
        [SEALED_MARK, _, _, a, b, c, d, ..] => Some(u32::from_be_bytes([*a, *b, *c, *d])),
        _ => None,
    }
}

fn value_aad(engine_key: &[u8], header: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(engine_key.len() + header.len());
    aad.extend_from_slice(engine_key);
    aad.extend_from_slice(header);
    aad
}

/// The unwrapped keyring: every DEK of the row in process memory, zeroed on
/// drop (`TG-ENC-001`).
pub struct Keyring {
    row: KeyringRow,
    deks: BTreeMap<u32, SecretKey>,
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyring")
            .field(
                "kek_fingerprint",
                &fingerprint_hex(&self.row.kek_fingerprint),
            )
            .field("key_ids", &self.deks.keys().collect::<Vec<_>>())
            .field("active", &self.row.active_id())
            .finish_non_exhaustive()
    }
}

/// Why a keyring was not unwrapped.
#[derive(Debug, thiserror::Error)]
#[error("key {key_id}: {reason}")]
pub struct UnwrapError {
    /// The first key that failed.
    pub key_id: u32,
    /// Why.
    pub reason: String,
}

impl Keyring {
    /// Unwraps every key of `row` with `provider`.
    ///
    /// # Errors
    ///
    /// The first key whose wrap does not authenticate.
    pub fn unwrap(row: KeyringRow, provider: &dyn MasterKeyProvider) -> Result<Self, UnwrapError> {
        let mut deks = BTreeMap::new();
        for key in &row.keys {
            let dek = provider
                .unwrap(key.id, &key.nonce, &key.wrapped)
                .map_err(|e| UnwrapError {
                    key_id: key.id,
                    reason: e.to_string(),
                })?;
            deks.insert(key.id, dek);
        }
        Ok(Self { row, deks })
    }

    /// A keyring with no key yet, under `provider`'s KEK.
    #[must_use]
    pub fn empty(provider: &dyn MasterKeyProvider) -> Self {
        Self {
            row: KeyringRow {
                version: KEYRING_VERSION,
                kek_fingerprint: provider.fingerprint(),
                next_key_id: 1,
                keys: Vec::new(),
            },
            deks: BTreeMap::new(),
        }
    }

    /// The row this keyring is persisted as.
    #[must_use]
    pub fn row(&self) -> &KeyringRow {
        &self.row
    }

    /// The active key's id.
    #[must_use]
    pub fn active_id(&self) -> Option<u32> {
        self.row.active_id()
    }

    /// Every key re-wrapped under `provider`'s KEK with fresh wrap nonces
    /// (`TG-ENC-007`).
    ///
    /// # Errors
    ///
    /// When the provider cannot wrap.
    pub fn rewrapped(self, provider: &dyn MasterKeyProvider) -> anyhow::Result<Self> {
        let Self { mut row, deks } = self;
        for key in &mut row.keys {
            let dek = deks
                .get(&key.id)
                .ok_or_else(|| anyhow::anyhow!("key {} is not unwrapped", key.id))?;
            (key.nonce, key.wrapped) = provider.wrap(key.id, dek)?;
        }
        row.kek_fingerprint = provider.fingerprint();
        Ok(Self { row, deks })
    }

    /// Adds a fresh DEK with id `next_key_id` as the active key, wrapped
    /// under `provider`'s KEK; any other key becomes decrypt-only.
    ///
    /// # Errors
    ///
    /// When the key id space is exhausted, or the random source or the
    /// provider fails.
    pub fn with_new_active(self, provider: &dyn MasterKeyProvider) -> anyhow::Result<Self> {
        let Self { mut row, mut deks } = self;
        let id = row.next_key_id;
        let next = id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("the key id space is exhausted"))?;
        let dek = generate_key()?;
        let (nonce, wrapped) = provider.wrap(id, &dek)?;
        for key in &mut row.keys {
            key.state = KeyState::DecryptOnly;
        }
        row.keys.push(WrappedKey {
            id,
            state: KeyState::Active,
            nonce,
            wrapped,
        });
        row.next_key_id = next;
        deks.insert(id, dek);
        Ok(Self { row, deks })
    }

    /// Every key decrypt-only: nothing is sealed any more.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        for key in &mut self.row.keys {
            key.state = KeyState::DecryptOnly;
        }
        self
    }

    /// Seals `plaintext` for the row `engine_key` under the active key, with
    /// a fresh random nonce.
    ///
    /// # Errors
    ///
    /// When there is no active key, or the random source fails.
    pub fn seal_value(&self, engine_key: &[u8], plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
        let id = self
            .active_id()
            .ok_or_else(|| anyhow::anyhow!("the keyring has no active key"))?;
        let dek = self
            .deks
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("key {id} is not unwrapped"))?;
        let nonce = random_bytes::<NONCE_LEN>()?;
        let mut sealed = Vec::with_capacity(plaintext.len() + ENVELOPE_OVERHEAD);
        sealed.extend_from_slice(&[SEALED_MARK, ENVELOPE_VERSION, CIPHER_XCHACHA20_POLY1305]);
        sealed.extend_from_slice(&id.to_be_bytes());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(plaintext);
        let aad = value_aad(engine_key, &sealed[..HEADER_LEN]);
        let tag = cipher(dek)
            .encrypt_in_place_detached(
                XNonce::from_slice(&nonce),
                &aad,
                &mut sealed[HEADER_LEN + NONCE_LEN..],
            )
            .map_err(|_| anyhow::anyhow!("sealing a value under key {id} failed"))?;
        sealed.extend_from_slice(&tag);
        Ok(sealed)
    }
}

/// The plaintext of the value `value` stored at `engine_key`: a plain value
/// as it is, a sealed one opened under `keyring` (`TG-ENC-002`).
///
/// # Errors
///
/// A [`ValueOpenError`]; never a silent skip.
pub fn open_value<'v>(
    keyring: Option<&Keyring>,
    engine_key: &[u8],
    value: &'v [u8],
) -> Result<std::borrow::Cow<'v, [u8]>, ValueOpenError> {
    if value.first() != Some(&SEALED_MARK) {
        return Ok(std::borrow::Cow::Borrowed(value));
    }
    if value.len() < ENVELOPE_OVERHEAD {
        return Err(ValueOpenError::Truncated { len: value.len() });
    }
    if value[1] != ENVELOPE_VERSION {
        return Err(ValueOpenError::UnknownVersion(value[1]));
    }
    if value[2] != CIPHER_XCHACHA20_POLY1305 {
        return Err(ValueOpenError::UnknownCipher(value[2]));
    }
    let id = u32::from_be_bytes([value[3], value[4], value[5], value[6]]);
    let dek = keyring
        .and_then(|k| k.deks.get(&id))
        .ok_or(ValueOpenError::UnknownKeyId(id))?;
    let (body, tag) = value.split_at(value.len() - TAG_LEN);
    let nonce = &body[HEADER_LEN..HEADER_LEN + NONCE_LEN];
    let mut plaintext = body[HEADER_LEN + NONCE_LEN..].to_vec();
    let aad = value_aad(engine_key, &value[..HEADER_LEN]);
    cipher(dek)
        .decrypt_in_place_detached(
            XNonce::from_slice(nonce),
            &aad,
            &mut plaintext,
            Tag::from_slice(tag),
        )
        .map_err(|_| {
            let field = |at: usize| {
                engine_key
                    .get(at..at + 4)
                    .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            };
            ValueOpenError::AuthenticationFailed {
                map_id: field(5),
                partition: field(1),
            }
        })?;
    Ok(std::borrow::Cow::Owned(plaintext))
}

/// HKDF info prefix of a WAL segment key; the header's first 57 bytes follow.
const WAL_SEGMENT_INFO: &[u8] = b"topgun/wal-segment/1";
/// The first 4 bytes of every frame nonce.
const FRAME_NONCE_PREFIX: &[u8; 4] = b"WALF";
/// Bytes a sealed frame payload adds to its plaintext: the tag.
pub const FRAME_TAG_LEN: usize = TAG_LEN;
/// Header bytes the segment key binds: all but its CRC.
const SEGMENT_HEADER_BODY_LEN: usize = 57;

/// A segment's key id is not in the keyring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("data key {0} is not in the keyring")]
pub struct UnknownSegmentKey(pub u32);

/// A sealed frame's tag does not match: a changed byte, or the frame opened
/// at another index or under another segment's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("frame authentication failed")]
pub struct FrameAuthFailed;

/// The key of one sealed WAL segment, zeroed on drop (`TG-ENC-003`,
/// `TG-ENC-005`): HKDF-SHA256 of the segment's DEK, salted with the header's
/// random salt, bound to every header byte but the CRC.
pub struct SegmentKey(SecretKey);

impl fmt::Debug for SegmentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentKey").finish_non_exhaustive()
    }
}

/// The nonce of the frame at `index`: `"WALF" ‖ 12 zero bytes ‖ index u64 BE`.
fn frame_nonce(index: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..4].copy_from_slice(FRAME_NONCE_PREFIX);
    nonce[16..].copy_from_slice(&index.to_be_bytes());
    nonce
}

impl SegmentKey {
    /// Seals `payload` in place as the frame at `index` of its segment, with
    /// `aad` the frame's first 9 bytes, and returns the tag. The nonce is the
    /// index, so the caller hands out each index of a segment once
    /// (`TG-ENC-005`).
    ///
    /// # Errors
    ///
    /// The cipher's refusal of an over-long payload.
    pub fn seal_frame(
        &self,
        index: u64,
        aad: &[u8],
        payload: &mut [u8],
    ) -> anyhow::Result<[u8; FRAME_TAG_LEN]> {
        let tag = cipher(&self.0)
            .encrypt_in_place_detached(XNonce::from_slice(&frame_nonce(index)), aad, payload)
            .map_err(|_| anyhow::anyhow!("sealing WAL frame {index} failed"))?;
        #[cfg(test)]
        test_frame_seals()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((self.test_fingerprint(), index));
        Ok(tag.into())
    }

    /// The plaintext of `sealed` (`ciphertext ‖ tag`), the frame at `index`
    /// with first 9 bytes `aad`.
    ///
    /// # Errors
    ///
    /// [`FrameAuthFailed`]; never a silent skip.
    pub fn open_frame(
        &self,
        index: u64,
        aad: &[u8],
        sealed: &[u8],
    ) -> Result<Vec<u8>, FrameAuthFailed> {
        let body_len = sealed.len().checked_sub(TAG_LEN).ok_or(FrameAuthFailed)?;
        let (body, tag) = sealed.split_at(body_len);
        let mut plaintext = body.to_vec();
        cipher(&self.0)
            .decrypt_in_place_detached(
                XNonce::from_slice(&frame_nonce(index)),
                aad,
                &mut plaintext,
                Tag::from_slice(tag),
            )
            .map_err(|_| FrameAuthFailed)?;
        Ok(plaintext)
    }

    /// A one-way fingerprint of the key, for the seal observer.
    #[cfg(test)]
    pub(crate) fn test_fingerprint(&self) -> Fingerprint {
        let digest = Sha256::new()
            .chain_update(b"topgun/test-segment-key-fp")
            .chain_update(&self.0[..])
            .finalize();
        let mut fingerprint = [0u8; 8];
        fingerprint.copy_from_slice(&digest[..8]);
        fingerprint
    }
}

/// `(segment key fingerprint, index)` of every frame sealed in this test
/// process, oldest first.
#[cfg(test)]
pub(crate) fn test_frame_seals() -> &'static std::sync::Mutex<Vec<(Fingerprint, u64)>> {
    static SEALS: std::sync::Mutex<Vec<(Fingerprint, u64)>> = std::sync::Mutex::new(Vec::new());
    &SEALS
}

impl Keyring {
    /// The key of the sealed segment whose header is `header`: HKDF-SHA256
    /// with IKM the DEK the header names, salt the header's salt and info
    /// `"topgun/wal-segment/1" ‖` the header's first 57 bytes.
    ///
    /// # Errors
    ///
    /// The header's key id is not in the keyring.
    ///
    /// # Panics
    ///
    /// Never: 32 bytes is a valid HKDF-SHA256 output length.
    pub fn segment_key(
        &self,
        header: &crate::storage::wal::segment::SegmentHeader,
    ) -> Result<SegmentKey, UnknownSegmentKey> {
        let dek = self
            .deks
            .get(&header.key_id)
            .ok_or(UnknownSegmentKey(header.key_id))?;
        Ok(derive_segment_key(
            dek,
            &header.salt,
            &header.encode()[..SEGMENT_HEADER_BODY_LEN],
        ))
    }
}

/// HKDF-SHA256 of `dek` with `salt`, info `"topgun/wal-segment/1" ‖ header_body`.
fn derive_segment_key(dek: &[u8; KEY_LEN], salt: &[u8], header_body: &[u8]) -> SegmentKey {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    hkdf::Hkdf::<Sha256>::new(Some(salt), dek)
        .expand_multi_info(&[WAL_SEGMENT_INFO, header_body], &mut key[..])
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    SegmentKey(key)
}

#[cfg(test)]
mod tests {
    //! `TG-ENC-001`, `TG-ENC-002`: the envelope, the keyring row and
    //! the DEK wrap.

    use std::collections::HashSet;

    use super::*;

    fn provider() -> FileKeyProvider {
        FileKeyProvider::from_kek(generate_key().expect("random"))
    }

    fn keyring(provider: &FileKeyProvider) -> Keyring {
        Keyring::empty(provider)
            .with_new_active(provider)
            .expect("new key")
    }

    /// `space ‖ partition ‖ map_id ‖ key`, as a record key is laid out.
    fn engine_key(space: u8, partition: u32, map_id: u32, key: &str) -> Vec<u8> {
        let mut bytes = vec![space];
        bytes.extend_from_slice(&partition.to_be_bytes());
        bytes.extend_from_slice(&map_id.to_be_bytes());
        bytes.extend_from_slice(key.as_bytes());
        bytes
    }

    #[test]
    fn a_sealed_value_opens_to_its_plaintext_and_carries_47_bytes_of_overhead() {
        let provider = provider();
        let keyring = keyring(&provider);
        let at = engine_key(0x10, 7, 1, "k");
        let plaintext = b"\x81\xa1v\x01 marker".to_vec();
        let sealed = keyring.seal_value(&at, &plaintext).expect("seal");
        assert_eq!(sealed.len(), plaintext.len() + 47);
        assert_eq!(&sealed[..7], &[0xC1, 1, 1, 0, 0, 0, 1]);
        assert_eq!(sealed_key_id(&sealed), Some(1));
        assert_eq!(
            open_value(Some(&keyring), &at, &sealed)
                .expect("open")
                .as_ref(),
            plaintext.as_slice()
        );
        // A plain value is returned as it is, with or without a keyring.
        assert_eq!(
            open_value(None, &at, &plaintext).expect("plain").as_ref(),
            plaintext.as_slice()
        );
    }

    #[test]
    fn flipping_any_byte_of_a_sealed_value_is_refused() {
        let provider = provider();
        let keyring = keyring(&provider);
        let at = engine_key(0x10, 3, 2, "key");
        let sealed = keyring.seal_value(&at, b"\x93\x01\x02\x03").expect("seal");
        for position in 0..sealed.len() {
            for bit in 0..8 {
                let mut changed = sealed.clone();
                changed[position] ^= 1 << bit;
                let opened = open_value(Some(&keyring), &at, &changed);
                // Byte 0 flipped makes a non-sealed value: plain values are
                // accepted on read (D10), so only the sealed bytes it held
                // come back, never the plaintext.
                if position == 0 {
                    assert_ne!(
                        opened.expect("a plain value").as_ref(),
                        b"\x93\x01\x02\x03",
                        "bit {bit}"
                    );
                    continue;
                }
                assert!(opened.is_err(), "position {position} bit {bit} opened");
            }
        }
    }

    #[test]
    fn a_sealed_value_moved_to_another_row_fails_authentication() {
        let provider = provider();
        let keyring = keyring(&provider);
        let at = engine_key(0x10, 5, 9, "user");
        let sealed = keyring.seal_value(&at, b"\xa3abc").expect("seal");
        for other in [
            engine_key(0x10, 5, 10, "user"),
            engine_key(0x10, 6, 9, "user"),
            engine_key(0x10, 5, 9, "uses"),
            engine_key(0x11, 5, 9, "user"),
        ] {
            assert!(matches!(
                open_value(Some(&keyring), &other, &sealed),
                Err(ValueOpenError::AuthenticationFailed { .. })
            ));
        }
        let error = open_value(Some(&keyring), &engine_key(0x10, 5, 10, "user"), &sealed)
            .expect_err("moved");
        assert_eq!(
            error.to_string(),
            "value authentication failed for map 10 partition 5"
        );
    }

    #[test]
    fn a_key_id_not_in_the_keyring_and_an_unknown_header_are_typed_errors() {
        let provider = provider();
        let keyring = keyring(&provider);
        let at = engine_key(0x10, 0, 1, "k");
        let sealed = keyring.seal_value(&at, b"\xc0").expect("seal");
        let mut other_id = sealed.clone();
        other_id[3..7].copy_from_slice(&2u32.to_be_bytes());
        assert_eq!(
            open_value(Some(&keyring), &at, &other_id),
            Err(ValueOpenError::UnknownKeyId(2))
        );
        assert_eq!(
            open_value(None, &at, &sealed),
            Err(ValueOpenError::UnknownKeyId(1))
        );
        let mut version = sealed.clone();
        version[1] = 2;
        assert_eq!(
            open_value(Some(&keyring), &at, &version),
            Err(ValueOpenError::UnknownVersion(2))
        );
        let mut cipher_byte = sealed.clone();
        cipher_byte[2] = 9;
        assert_eq!(
            open_value(Some(&keyring), &at, &cipher_byte),
            Err(ValueOpenError::UnknownCipher(9))
        );
        assert_eq!(
            open_value(Some(&keyring), &at, &sealed[..46]),
            Err(ValueOpenError::Truncated { len: 46 })
        );
    }

    #[test]
    fn a_thousand_seals_of_one_value_never_repeat_a_nonce() {
        let provider = provider();
        let keyring = keyring(&provider);
        let at = engine_key(0x10, 1, 1, "k");
        let mut nonces = HashSet::new();
        for _ in 0..1000 {
            let sealed = keyring.seal_value(&at, b"\x01").expect("seal");
            assert!(nonces.insert(sealed[7..31].to_vec()), "a nonce repeated");
        }
    }

    #[test]
    fn the_keyring_row_round_trips_and_its_strict_checks_refuse() {
        let provider = provider();
        let keyring = keyring(&provider)
            .with_new_active(&provider)
            .expect("second");
        let row = keyring.row().clone();
        assert_eq!(row.next_key_id, 3);
        assert_eq!(row.active_id(), Some(2));
        assert_eq!(row.keys[0].state, KeyState::DecryptOnly);
        assert_eq!(row.kek_fingerprint, provider.fingerprint());
        let bytes = row.encode().expect("encode");
        assert_eq!(KeyringRow::decode(&bytes).expect("decode"), row);

        let refused = |change: &dyn Fn(&mut KeyringRow)| {
            let mut bad = row.clone();
            change(&mut bad);
            KeyringRow::decode(&bad.encode().expect("encode")).expect_err("refused")
        };
        assert!(refused(&|r| r.version = 2).contains("version 2"));
        assert!(refused(&|r| r.keys[0].id = 0).contains("key id 0"));
        assert!(refused(&|r| r.keys[0].id = 2).contains("twice"));
        assert!(refused(&|r| r.keys[0].state = KeyState::Active).contains("2 active"));
        assert!(refused(&|r| r.next_key_id = 2).contains("not above"));
        assert!(KeyringRow::decode(b"\x01\x02").is_err());
    }

    #[test]
    fn a_wrap_opens_only_under_its_own_id_and_kek() {
        let provider = provider();
        let dek = generate_key().expect("random");
        let (nonce, wrapped) = provider.wrap(4, &dek).expect("wrap");
        assert_eq!(*provider.unwrap(4, &nonce, &wrapped).expect("unwrap"), *dek);
        assert!(provider.unwrap(5, &nonce, &wrapped).is_err());
        assert!(self::provider().unwrap(4, &nonce, &wrapped).is_err());
        // A keyring unwraps under its own KEK only; the error names the key.
        let keyring = keyring(&provider);
        let error = Keyring::unwrap(keyring.row().clone(), &self::provider()).expect_err("other");
        assert_eq!(error.key_id, 1);
        let reopened = Keyring::unwrap(keyring.row().clone(), &provider).expect("same");
        let at = engine_key(0x10, 1, 1, "k");
        let sealed = keyring.seal_value(&at, b"\x02").expect("seal");
        assert_eq!(
            open_value(Some(&reopened), &at, &sealed)
                .expect("open")
                .as_ref(),
            b"\x02"
        );
    }

    #[test]
    fn a_rewrapped_keyring_opens_under_the_new_kek_only() {
        let old = provider();
        let new = provider();
        let keyring = keyring(&old);
        let rewrapped = keyring.rewrapped(&new).expect("rewrap");
        assert_eq!(rewrapped.row().kek_fingerprint, new.fingerprint());
        assert!(Keyring::unwrap(rewrapped.row().clone(), &old).is_err());
        assert!(Keyring::unwrap(rewrapped.row().clone(), &new).is_ok());
    }

    #[test]
    fn the_seal_pass_row_round_trips_and_refuses_an_unknown_direction() {
        let pass = SealPass {
            direction: PassDirection::Unseal,
            cursor: vec![0x10, 1, 2],
        };
        assert_eq!(SealPass::decode(&pass.encode()).expect("decode"), pass);
        assert!(SealPass::decode(&[0x03]).is_err());
        assert!(SealPass::decode(&[]).is_err());
    }

    fn key_file(dir: &Path, name: &str, content: &[u8], mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        }
        path
    }

    #[test]
    fn the_file_provider_reads_64_hex_characters_and_refuses_everything_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hex_kek = hex::encode(*generate_key().expect("random"));
        let good = key_file(dir.path(), "good", format!("{hex_kek}\n").as_bytes(), 0o600);
        let loaded = FileKeyProvider::load(&good).expect("good");
        let bare = key_file(dir.path(), "bare", hex_kek.as_bytes(), 0o400);
        assert_eq!(
            FileKeyProvider::load(&bare).expect("bare").fingerprint(),
            loaded.fingerprint()
        );
        for content in [
            &hex_kek[..63],
            &format!("{hex_kek}0"),
            &format!("{hex_kek}\n\n"),
        ] {
            let path = key_file(dir.path(), "bad", content.as_bytes(), 0o600);
            assert!(matches!(
                FileKeyProvider::load(&path),
                Err(KeyFileError::Malformed { .. })
            ));
        }
        let mut not_hex = hex_kek.clone().into_bytes();
        not_hex[10] = b'g';
        let path = key_file(dir.path(), "nothex", &not_hex, 0o600);
        let error = FileKeyProvider::load(&path).expect_err("not hex");
        assert!(matches!(error, KeyFileError::Malformed { .. }));
        assert!(!error.to_string().contains(&hex_kek[..8]));
        assert!(matches!(
            FileKeyProvider::load(&dir.path().join("missing")),
            Err(KeyFileError::Unreadable { .. })
        ));
        #[cfg(unix)]
        {
            let open = key_file(dir.path(), "open", hex_kek.as_bytes(), 0o640);
            assert!(matches!(
                FileKeyProvider::load(&open),
                Err(KeyFileError::TooPermissive { mode: 0o640, .. })
            ));
            // Judged on the target, as a Kubernetes Secret mount is a symlink.
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&open, &link).expect("symlink");
            assert!(matches!(
                FileKeyProvider::load(&link),
                Err(KeyFileError::TooPermissive { .. })
            ));
        }
    }

    /// A sealed segment header for `partition` from `first_seq` under `key_id`.
    fn sealed_header(
        key_id: u32,
        partition: u32,
        first_seq: u64,
    ) -> crate::storage::wal::segment::SegmentHeader {
        let mut header =
            crate::storage::wal::segment::SegmentHeader::new_unsealed(partition, first_seq);
        header.cipher = crate::storage::wal::segment::SegmentCipher::XChaCha20Poly1305;
        header.key_id = key_id;
        header
    }

    /// AC2 (`TG-ENC-003`, `TG-ENC-005`): a sealed frame opens at its index
    /// under its segment's key, and at no other index, under no key of a
    /// header that differs in any one byte the key binds — salt, partition,
    /// first sequence, layout, schema, cipher, key id — and not with any
    /// changed byte of its AAD or of the sealed payload.
    #[test]
    fn ac2_a_sealed_frame_opens_only_at_its_index_under_its_segment_key() {
        let provider = provider();
        let keyring = keyring(&provider);
        let id = keyring.active_id().expect("active");
        let header = sealed_header(id, 7, 42);
        let key = keyring.segment_key(&header).expect("key");
        let aad = b"TGWL\x02\x00\x00\x00\x20";
        let plaintext = b"\x81\xa1k\xa3abc a frame payload".to_vec();
        let mut sealed = plaintext.clone();
        let tag = key.seal_frame(5, aad, &mut sealed).expect("seal");
        assert_ne!(sealed, plaintext);
        sealed.extend_from_slice(&tag);
        assert_eq!(key.open_frame(5, aad, &sealed), Ok(plaintext));
        for other in [0, 4, 6, u64::MAX] {
            assert_eq!(key.open_frame(other, aad, &sealed), Err(FrameAuthFailed));
        }

        let dek = &keyring.deks[&id];
        let body = header.encode();
        let body = &body[..SEGMENT_HEADER_BODY_LEN];
        let same = derive_segment_key(dek, &header.salt, body);
        assert_eq!(
            same.open_frame(5, aad, &sealed).map(|p| p.len()),
            Ok(sealed.len() - 16)
        );
        // Bytes 0..5 are the magic and the header version; every byte after
        // them names a field, and the salt is also the HKDF salt.
        for at in 0..SEGMENT_HEADER_BODY_LEN {
            let mut changed = body.to_vec();
            changed[at] ^= 0x01;
            let salt = &changed[13..45];
            let key = derive_segment_key(dek, salt, &changed);
            assert_eq!(
                key.open_frame(5, aad, &sealed),
                Err(FrameAuthFailed),
                "header byte {at} changed"
            );
        }
        // Each field through the header type too.
        let mut fields = vec![
            sealed_header(id, 8, 42),
            sealed_header(id, 7, 43),
            sealed_header(id, 7, 42),
        ];
        fields[2].salt = header.salt;
        fields[2].salt[31] ^= 1;
        for other in fields {
            let key = keyring.segment_key(&other).expect("key");
            assert_eq!(key.open_frame(5, aad, &sealed), Err(FrameAuthFailed));
        }
        // Another key id names another DEK.
        let rotated = keyring.with_new_active(&provider).expect("new key");
        let mut other_id = header;
        other_id.key_id = rotated.active_id().expect("active");
        let key = rotated.segment_key(&other_id).expect("key");
        assert_eq!(key.open_frame(5, aad, &sealed), Err(FrameAuthFailed));
        let key = rotated.segment_key(&header).expect("the old id opens");
        assert!(key.open_frame(5, aad, &sealed).is_ok());

        for at in 0..aad.len() {
            let mut changed = *aad;
            changed[at] ^= 0x01;
            assert_eq!(key.open_frame(5, &changed, &sealed), Err(FrameAuthFailed));
        }
        for at in 0..sealed.len() {
            let mut changed = sealed.clone();
            changed[at] ^= 0x01;
            assert_eq!(key.open_frame(5, aad, &changed), Err(FrameAuthFailed));
        }
        assert_eq!(key.open_frame(5, aad, &sealed[..15]), Err(FrameAuthFailed));
        assert_eq!(
            keyring_without(&rotated, id).segment_key(&header).err(),
            Some(UnknownSegmentKey(id))
        );
    }

    /// `keyring` without key `id`.
    fn keyring_without(keyring: &Keyring, id: u32) -> Keyring {
        let mut deks = BTreeMap::new();
        for (k, dek) in &keyring.deks {
            if *k != id {
                deks.insert(*k, dek.clone());
            }
        }
        Keyring {
            row: keyring.row.clone(),
            deks,
        }
    }

    /// AC3 (`TG-ENC-005`): two headers created in a row for one
    /// `(partition, first_seq)` carry different salts and derive different
    /// keys, so a file replaced under its name never reuses a nonce.
    #[test]
    fn ac3_two_headers_for_one_name_have_different_salts_and_keys() {
        let provider = provider();
        let keyring = keyring(&provider);
        let id = keyring.active_id().expect("active");
        let first = sealed_header(id, 3, 0);
        let second = sealed_header(id, 3, 0);
        assert_ne!(first.salt, second.salt);
        let a = keyring.segment_key(&first).expect("key");
        let b = keyring.segment_key(&second).expect("key");
        assert_ne!(a.test_fingerprint(), b.test_fingerprint());
        let aad = b"TGWL\x02\x00\x00\x00\x11";
        let mut sealed = b"one frame".to_vec();
        let tag = a.seal_frame(0, aad, &mut sealed).expect("seal");
        sealed.extend_from_slice(&tag);
        assert_eq!(b.open_frame(0, aad, &sealed), Err(FrameAuthFailed));
    }
}
