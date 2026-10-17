//! Opens `.pack` files and indexes the textures inside them.

use anyhow::{
    Context,
    Result,
    bail,
};
use libpzmap::texturepack::{
    Texture,
    TexturePack,
};
use memmap2::Mmap;
use std::{
    ffi::OsStr,
    fs::File,
    path::{
        Path,
        PathBuf,
    },
};

/// Largest `.pack` this viewer will open, well above any real texture pack.
const MAX_PACK_BYTES: u64 = 8 << 30;

/// Memory-maps a `.pack` file, refusing one larger than [`MAX_PACK_BYTES`].
fn map_pack(path: &Path) -> Result<Mmap> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("measuring {}", path.display()))?
        .len();
    if len > MAX_PACK_BYTES {
        bail!(
            "{} is {len} bytes, over the {MAX_PACK_BYTES} byte limit",
            path.display()
        );
    }
    // SAFETY: the viewer only reads the file. If another process shortens the file while it is
    // mapped, the viewer is killed with SIGBUS. That is acceptable for an interactive tool.
    unsafe { Mmap::map(&file) }.with_context(|| format!("mapping {}", path.display()))
}

/// Where a texture lives in the parsed pack.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextureRef {
    /// Index of the page.
    pub page: usize,
    /// Index of the texture within its page.
    pub texture: usize,
}

/// One open pack, with every texture in it indexed by name.
pub struct Pack {
    /// The file name.
    pub name: String,
    /// The path the pack was opened from.
    pub path: PathBuf,
    pack: TexturePack<'static>,
    /// Every texture in the pack with its lowercased name, sorted by that name.
    index: Vec<(String, TextureRef)>,
}

impl Pack {
    /// Memory-maps `path` and parses it.
    ///
    /// The parsed pack borrows the mapped bytes, and a struct cannot hold both the bytes and a
    /// borrow of them. So the mapping is leaked: it is never unmapped, and the parsed pack can
    /// borrow it for the rest of the program.
    pub fn open(path: &Path) -> Result<Self> {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        let bytes: &'static Mmap = Box::leak(Box::new(map_pack(path)?));

        let pack =
            TexturePack::parse(bytes).with_context(|| format!("parsing {}", path.display()))?;

        let mut index: Vec<(String, TextureRef)> =
            pack.pages
                .iter()
                .enumerate()
                .flat_map(|(page, p)| {
                    p.textures.iter().enumerate().map(move |(texture, t)| {
                        (t.name.to_lowercase(), TextureRef { page, texture })
                    })
                })
                .collect();
        index.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        Ok(Self {
            name,
            path: path.to_path_buf(),
            pack,
            index,
        })
    }

    /// The pack's pages, in file order.
    pub fn pages(&self) -> &[libpzmap::texturepack::Page<'static>] {
        &self.pack.pages
    }

    /// How many pages the pack has.
    pub fn page_count(&self) -> usize {
        self.pack.pages.len()
    }

    /// How many textures the pack has across all its pages.
    pub fn texture_count(&self) -> usize {
        self.pack.texture_count()
    }

    /// The page at `index`, if there is one.
    pub fn page(&self, index: usize) -> Option<&libpzmap::texturepack::Page<'static>> {
        self.pack.pages.get(index)
    }

    /// The texture at `at`, if there is one.
    pub fn texture(&self, at: TextureRef) -> Option<&Texture<'static>> {
        self.pack.pages.get(at.page)?.textures.get(at.texture)
    }

    /// Every texture whose name contains `needle`, ignoring case, in name order. An empty
    /// `needle` matches nothing.
    pub fn search(&self, needle: &str) -> Vec<TextureRef> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle = needle.to_lowercase();
        self.index
            .iter()
            .filter(|(name, _)| name.contains(&needle))
            .map(|(_, at)| *at)
            .collect()
    }
}

/// What a path given to the viewer turned out to be.
pub enum Target {
    /// A single `.pack` file.
    File(PathBuf),
    /// The `.pack` files in a folder, in name order. There is at least one.
    Directory(Vec<PathBuf>),
}

/// Works out whether `path` is a `.pack` file or a folder, and lists the packs in a folder.
///
/// Subfolders are not searched, since the game keeps all its texture packs directly in
/// `media/texturepacks`.
pub fn resolve(path: &Path) -> Result<Target> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;

    if meta.is_file() {
        return Ok(Target::File(path.to_path_buf()));
    }

    let mut packs: Vec<PathBuf> = std::fs::read_dir(path)
        .with_context(|| format!("listing {}", path.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().and_then(OsStr::to_str) == Some("pack"))
        .collect();

    if packs.is_empty() {
        bail!("no .pack files in {}", path.display());
    }
    packs.sort_unstable();
    Ok(Target::Directory(packs))
}
