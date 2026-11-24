//! Pet service — port of helper/pets.py.
//!
//! Builtin designs render from renderer/pets/<id>.webp. Installing a pet
//! writes ~/.codex/pets/<id>/ with the spritesheet + manifest, like the
//! original's install(). Selection state mirrors the original's pet.json.

use anyhow::{bail, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::catalog::pets as builtin_designs;
use crate::fsutil::{atomic_write, atomic_write_bytes};

/// helper/../renderer/pets — repo layout during dev builds.
fn assets_dir() -> PathBuf {
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("renderer")
        .join("pets");
    if bundled.is_dir() {
        return bundled;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("renderer").join("pets");
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    bundled
}

fn pet_id_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[a-z0-9-]{3,64}$").unwrap())
}

pub struct PetService {
    dir: PathBuf,
    config: PathBuf,
    assets: PathBuf,
}
impl PetService {
    pub fn new(home: PathBuf) -> Self {
        let codex_home = std::env::var("CODEX_HOME")
            .unwrap_or_default()
            .trim()
            .to_string();
        let codex_home = if codex_home.is_empty() {
            home.join(".codex")
        } else {
            PathBuf::from(codex_home)
        };
        Self {
            dir: codex_home.join("pets"),
            config: codex_home.join("pets").join("pet.json"),
            assets: assets_dir(),
        }
    }

    pub fn list(&self) -> Value {
        let mut pets: Vec<Value> = builtin_designs()
            .into_iter()
            .map(|mut pet| {
                pet["installed"] = json!(false);
                pet["source"] = json!("builtin");
                pet["spriteVersion"] = json!(1);
                pet
            })
            .collect();
        if self.dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&self.dir) {
                for entry in entries.flatten() {
                    let id = entry.file_name().to_string_lossy().into_owned();
                    if !entry.path().is_dir()
                        || !pet_id_re().is_match(&id)
                        || id == "__selected__"
                    {
                        continue;
                    }
                    let manifest = self.manifest(&id);
                    if let Some(builtin) = pets
                        .iter_mut()
                        .find(|p| p["id"].as_str() == Some(id.as_str()))
                    {
                        builtin["installed"] = json!(true);
                    } else if let Some(manifest) = manifest {
                        pets.push(json!({
                            "id": id,
                            "displayName": manifest["displayName"].as_str()
                                .filter(|s| !s.is_empty()).unwrap_or(id.as_str()),
                            "description": manifest["description"].as_str().unwrap_or_default(),
                            "tags": manifest["tags"].as_array().cloned().unwrap_or_default(),
                            "source": "local",
                            "installed": true,
                            "spriteVersion": if crate::tpapi::truthy(&manifest["spriteVersion"]) {
                                manifest["spriteVersion"].clone()
                            } else { json!(1) },
                        }));
                    }
                }
            }
        }
        json!({"pets": pets, "selectedId": self.selected()})
    }

    pub fn use_pet(&self, id: &str) -> Result<Value> {
        if !pet_id_re().is_match(id) {
            bail!("无效的宠物编号");
        }
        std::fs::create_dir_all(&self.dir)?;
        // install builtin sprite if not present
        let target = self.dir.join(id);
        std::fs::create_dir_all(&target)?;
        let designs = builtin_designs();
        let design = designs.iter().find(|p| p["id"].as_str() == Some(id));
        let src = self.assets.join(format!("{id}.webp"));
        if design.is_some() && src.exists() {
            let sheet = target.join("spritesheet.webp");
            if !sheet.exists() {
                std::fs::copy(&src, &sheet)?;
            }
            let design = design.unwrap();
            atomic_write(
                &target.join("manifest.json"),
                &crate::fsutil::json_text(&json!({
                    "id": id,
                    "displayName": design["displayName"],
                    "description": design["description"],
                    "spriteVersion": 1,
                    "spritesheetPath": "spritesheet.webp",
                })),
            )?;
        }
        atomic_write(
            &self.config,
            &crate::fsutil::json_text(&json!({"selectedPet": id})),
        )?;
        Ok(self.list())
    }

    pub fn preview(&self, id: &str) -> Result<Value> {
        if !pet_id_re().is_match(id) {
            bail!("无效的宠物编号");
        }
        for path in [
            self.dir.join(id).join("spritesheet.webp"),
            self.assets.join(format!("{id}.webp")),
        ] {
            if path.exists() {
                let data = base64::engine::general_purpose::STANDARD
                    .encode(std::fs::read(&path)?);
                return Ok(json!(format!("data:image/webp;base64,{data}")));
            }
        }
        bail!("没有找到宠物预览图")
    }

    /// Import a .codex-pet.zip: copy spritesheet + manifest.
    pub fn import_zip(&self, path: &str) -> Result<Value> {
        let archive = ziputil::read(Path::new(path))?;
        let manifest_name = archive
            .keys()
            .find(|n| n.ends_with("manifest.json"))
            .cloned();
        let manifest_name = match manifest_name {
            Some(name) => name,
            None => bail!("宠物包缺少 manifest.json"),
        };
        let mut manifest: Value = serde_json::from_str(
            &String::from_utf8_lossy(&archive[&manifest_name]),
        )?;
        let pet_id = manifest["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| {
                Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().replace(".codex-pet", ""))
                    .unwrap_or_default()
            });
        if !pet_id_re().is_match(&pet_id) {
            bail!("宠物包编号无效");
        }
        let sheet = archive
            .keys()
            .find(|n| {
                let lower = n.to_lowercase();
                lower.ends_with(".webp") || lower.ends_with(".png")
            })
            .cloned();
        let target = self.dir.join(&pet_id);
        std::fs::create_dir_all(&target)?;
        if let Some(sheet) = sheet {
            let ext = if sheet.to_lowercase().ends_with(".png") {
                ".png"
            } else {
                ".webp"
            };
            std::fs::write(target.join(format!("spritesheet{ext}")), &archive[&sheet])?;
            manifest["spritesheetPath"] = json!(format!("spritesheet{ext}"));
        }
        atomic_write(
            &target.join("manifest.json"),
            &crate::fsutil::json_text(&manifest),
        )?;
        self.use_pet(&pet_id)
    }

    /// Pack the builtin spritesheet as a .codex-pet.zip.
    pub fn download_zip(&self, id: &str, target: &str) -> Result<()> {
        let designs = builtin_designs();
        let design = designs.iter().find(|p| p["id"].as_str() == Some(id));
        let src = self.assets.join(format!("{id}.webp"));
        if design.is_none() || !src.exists() {
            bail!("没有找到这个宠物包");
        }
        let design = design.unwrap();
        // json.dumps(manifest, indent=2) — no trailing newline in the zip.
        let manifest = format!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "id": design["id"],
                "displayName": design["displayName"],
                "description": design["description"],
                "tags": design["tags"],
                "spriteVersion": 1,
                "spritesheetPath": "spritesheet.webp",
            }))?
        );
        let bytes = ziputil::write(&[
            (format!("{id}/manifest.json"), manifest.into_bytes()),
            (format!("{id}/spritesheet.webp"), std::fs::read(&src)?),
        ]);
        atomic_write_bytes(Path::new(target), &bytes)
    }

    fn selected(&self) -> Option<String> {
        let data: Value =
            serde_json::from_str(&std::fs::read_to_string(&self.config).ok()?).ok()?;
        let selected = data["selectedPet"].as_str().unwrap_or_default();
        if pet_id_re().is_match(selected) {
            Some(selected.to_string())
        } else {
            None
        }
    }

    fn manifest(&self, id: &str) -> Option<Value> {
        serde_json::from_str(
            &std::fs::read_to_string(self.dir.join(id).join("manifest.json")).ok()?,
        )
        .ok()
    }
}

/// Minimal ZIP read/write — Cargo.toml has no zip crate, so .codex-pet.zip
/// handling is implemented here. Reads method 0 (store) and 8 (deflate,
/// via a compact inflate decoder); writes method 0 archives (valid for
/// any zip reader, Python zipfile included).
mod ziputil {
    use anyhow::{bail, Result};
    use std::collections::HashMap;
    use std::path::Path;

    pub fn read(path: &Path) -> Result<HashMap<String, Vec<u8>>> {
        let data = std::fs::read(path)?;
        read_bytes(&data)
    }

    fn u16le(data: &[u8], at: usize) -> usize {
        data.get(at..at + 2)
            .map(|b| b[0] as usize | ((b[1] as usize) << 8))
            .unwrap_or(0)
    }

    fn u32le(data: &[u8], at: usize) -> usize {
        data.get(at..at + 4)
            .map(|b| {
                b[0] as usize
                    | ((b[1] as usize) << 8)
                    | ((b[2] as usize) << 16)
                    | ((b[3] as usize) << 24)
            })
            .unwrap_or(0)
    }

    pub fn read_bytes(data: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
        // locate End Of Central Directory (PK\x05\x06) in the last 64KiB
        let scan_from = data.len().saturating_sub(66_000);
        let mut eocd = None;
        for i in (scan_from..data.len().saturating_sub(3)).rev() {
            if &data[i..i + 4] == b"PK\x05\x06" {
                eocd = Some(i);
                break;
            }
        }
        let eocd = eocd.ok_or_else(|| anyhow::anyhow!("宠物包不是有效的 zip 文件"))?;
        let count = u16le(data, eocd + 10);
        let mut at = u32le(data, eocd + 16);
        let mut entries = Vec::new();
        for _ in 0..count {
            if data.get(at..at + 4) != Some(b"PK\x01\x02") {
                break;
            }
            let method = u16le(data, at + 10);
            let comp_size = u32le(data, at + 20);
            let name_len = u16le(data, at + 28);
            let extra_len = u16le(data, at + 30);
            let comment_len = u16le(data, at + 32);
            let local = u32le(data, at + 42);
            let name = String::from_utf8_lossy(&data[at + 46..at + 46 + name_len])
                .into_owned();
            entries.push((name, method, comp_size, local));
            at += 46 + name_len + extra_len + comment_len;
        }
        let mut out = HashMap::new();
        for (name, method, comp_size, local) in entries {
            if data.get(local..local + 4) != Some(b"PK\x03\x04") {
                continue;
            }
            let name_len = u16le(data, local + 26);
            let extra_len = u16le(data, local + 28);
            let start = local + 30 + name_len + extra_len;
            let end = (start + comp_size).min(data.len());
            let raw = &data[start..end];
            let content = match method {
                0 => raw.to_vec(),
                8 => inflate(raw)?,
                _ => bail!("宠物包使用了不支持的压缩方式"),
            };
            out.insert(name, content);
        }
        Ok(out)
    }

    // ---- store-method writer (valid zip, uncompressed entries) ----

    pub fn write(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&20u16.to_le_bytes()); // version
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // method 0
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&20u16.to_le_bytes()); // made by
            central.extend_from_slice(&20u16.to_le_bytes()); // need
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk
            central.extend_from_slice(&0u16.to_le_bytes()); // int attr
            central.extend_from_slice(&0u32.to_le_bytes()); // ext attr
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, cell) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
            }
            *cell = c;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    // ---- raw deflate decoder (RFC 1951), puff.c style ----

    const LBASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
        67, 83, 99, 115, 131, 163, 195, 227, 258,
    ];
    const LEXT: [u32; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4,
        5, 5, 5, 5, 0,
    ];
    const DBASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513,
        769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DEXT: [u32; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10,
        10, 11, 11, 12, 12, 13, 13,
    ];
    const CLC_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];

    struct Bits<'a> {
        data: &'a [u8],
        pos: usize,
    }
    impl<'a> Bits<'a> {
        fn bits(&mut self, n: u32) -> u32 {
            let mut v = 0u32;
            for i in 0..n {
                let byte = self.data.get(self.pos / 8).copied().unwrap_or(0);
                v |= (((byte >> (self.pos % 8)) & 1) as u32) << i;
                self.pos += 1;
            }
            v
        }
    }

    struct Huffman {
        counts: [usize; 16],
        symbols: Vec<u16>,
    }
    impl Huffman {
        fn new(lengths: &[u8]) -> Self {
            let mut counts = [0usize; 16];
            for &len in lengths {
                counts[len as usize] += 1;
            }
            counts[0] = 0;
            let mut offsets = [0usize; 16];
            for len in 1..16 {
                offsets[len] = offsets[len - 1] + counts[len - 1];
            }
            let mut symbols = vec![0u16; lengths.iter().filter(|&&l| l != 0).count()];
            for (symbol, &len) in lengths.iter().enumerate() {
                if len != 0 {
                    symbols[offsets[len as usize]] = symbol as u16;
                    offsets[len as usize] += 1;
                }
            }
            Huffman { counts, symbols }
        }
        fn decode(&self, bits: &mut Bits) -> Result<u16> {
            let mut code = 0usize;
            let mut first = 0usize;
            let mut index = 0usize;
            for len in 1..16 {
                code |= bits.bits(1) as usize;
                let count = self.counts[len];
                if code - first < count {
                    return Ok(self.symbols[index + (code - first)]);
                }
                index += count;
                first = (first + count) << 1;
                code <<= 1;
            }
            bail!("宠物包数据损坏")
        }
    }

    fn inflate(data: &[u8]) -> Result<Vec<u8>> {
        let mut bits = Bits { data, pos: 0 };
        let mut out: Vec<u8> = Vec::new();
        loop {
            let last = bits.bits(1);
            let btype = bits.bits(2);
            match btype {
                0 => {
                    // stored block: skip to byte boundary
                    bits.pos = (bits.pos + 7) & !7;
                    let start = bits.pos / 8;
                    let len = u16le(data, start);
                    bits.pos += 32;
                    if start + 4 + len > data.len() {
                        bail!("宠物包数据损坏");
                    }
                    out.extend_from_slice(&data[start + 4..start + 4 + len]);
                }
                1 => {
                    let mut lit_len = [0u8; 288];
                    lit_len[..144].fill(8);
                    lit_len[144..256].fill(9);
                    lit_len[256..280].fill(7);
                    lit_len[280..288].fill(8);
                    let lit = Huffman::new(&lit_len);
                    let dist = Huffman::new(&[5u8; 30]);
                    inflate_block(&mut bits, &lit, &dist, &mut out)?;
                }
                2 => {
                    let (lit, dist) = dynamic_tables(&mut bits)?;
                    inflate_block(&mut bits, &lit, &dist, &mut out)?;
                }
                _ => bail!("宠物包数据损坏"),
            }
            if last == 1 {
                return Ok(out);
            }
        }
    }

    fn dynamic_tables(bits: &mut Bits) -> Result<(Huffman, Huffman)> {
        let hlit = bits.bits(5) as usize + 257;
        let hdist = bits.bits(5) as usize + 1;
        let hclen = bits.bits(4) as usize + 4;
        if hlit > 286 || hdist > 30 {
            bail!("宠物包数据损坏");
        }
        let mut cl_lengths = [0u8; 19];
        for i in 0..hclen {
            cl_lengths[CLC_ORDER[i]] = bits.bits(3) as u8;
        }
        let cl = Huffman::new(&cl_lengths);
        let total = hlit + hdist;
        let mut lengths = vec![0u8; total];
        let mut i = 0;
        while i < total {
            let sym = cl.decode(bits)?;
            match sym {
                0..=15 => {
                    lengths[i] = sym as u8;
                    i += 1;
                }
                16 => {
                    if i == 0 {
                        bail!("宠物包数据损坏");
                    }
                    let prev = lengths[i - 1];
                    for _ in 0..3 + bits.bits(2) {
                        if i >= total {
                            bail!("宠物包数据损坏");
                        }
                        lengths[i] = prev;
                        i += 1;
                    }
                }
                17 | 18 => {
                    let zeros = if sym == 17 {
                        3 + bits.bits(3)
                    } else {
                        11 + bits.bits(7)
                    };
                    i += zeros as usize;
                    if i > total {
                        bail!("宠物包数据损坏");
                    }
                }
                _ => bail!("宠物包数据损坏"),
            }
        }
        let lit = Huffman::new(&lengths[..hlit]);
        let dist = Huffman::new(&lengths[hlit..]);
        Ok((lit, dist))
    }

    fn inflate_block(
        bits: &mut Bits,
        lit: &Huffman,
        dist: &Huffman,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        loop {
            let sym = lit.decode(bits)?;
            match sym {
                0..=255 => out.push(sym as u8),
                256 => return Ok(()),
                257..=285 => {
                    let index = (sym - 257) as usize;
                    let length = LBASE[index] as usize + bits.bits(LEXT[index]) as usize;
                    let dsym = dist.decode(bits)? as usize;
                    if dsym >= 30 {
                        bail!("宠物包数据损坏");
                    }
                    let distance =
                        DBASE[dsym] as usize + bits.bits(DEXT[dsym]) as usize;
                    if distance > out.len() {
                        bail!("宠物包数据损坏");
                    }
                    let start = out.len() - distance;
                    for k in 0..length {
                        let b = out[start + k];
                        out.push(b);
                    }
                }
                _ => bail!("宠物包数据损坏"),
            }
        }
    }
}
