//! The Typst [`World`] of one PDF render: the generated main source, the files it
//! names (images, diagrams, the `overrides.pdf` template and what that template reads
//! next to itself), the bundled fonts and a fixed date. Nothing else on disk is
//! reachable, and there are no packages, so a render never touches the network.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use typst::diag::{FileError, FileResult, PackageError};
use typst::foundations::{Bytes, Datetime, Dict, Duration, IntoValue};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

/// The virtual path of the generated document.
pub(super) const MAIN: &str = "/main.typ";
/// The virtual directory the template's directory is mounted at.
pub(super) const TEMPLATE_DIR: &str = "/template";

/// The fonts bundled with Typst (`typst-assets`), parsed once.
fn bundled_fonts() -> &'static [Font] {
    static FONTS: OnceLock<Vec<Font>> = OnceLock::new();
    FONTS.get_or_init(|| {
        typst_assets::fonts()
            .flat_map(|data| Font::iter(Bytes::new(data)))
            .collect()
    })
}

/// The names of the font families bundled with Typst, sorted.
pub fn bundled_font_families() -> Vec<String> {
    let mut families: Vec<String> = bundled_fonts()
        .iter()
        .map(|font| font.info().family.clone())
        .collect();
    families.sort();
    families.dedup();
    families
}

/// The content of a virtual file: an image read from disk or a rendered diagram.
#[derive(Debug, Clone)]
pub(super) enum Asset {
    Bytes(Bytes),
}

/// The world of one compilation.
pub(super) struct MrqWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
    /// Virtual path (with a leading `/`) → content.
    assets: HashMap<String, Asset>,
    /// The template's directory on disk, mounted at [`TEMPLATE_DIR`].
    template_dir: Option<PathBuf>,
    today: Option<Datetime>,
    /// Files read so far, so that every access sees the same bytes.
    read: Mutex<HashMap<FileId, FileResult<Bytes>>>,
}

/// The file id of the virtual path `path` in the project.
pub(super) fn file_id(path: &str) -> Option<FileId> {
    let vpath = VirtualPath::new(path).ok()?;
    Some(RootedPath::new(VirtualRoot::Project, vpath).intern())
}

impl MrqWorld {
    /// A world for `source`, with `assets` and the template directory mounted, the
    /// bundled fonts and `today` as the date. `inputs` become `sys.inputs`.
    pub(super) fn new(
        source: String,
        assets: HashMap<String, Asset>,
        template_dir: Option<PathBuf>,
        inputs: Vec<(&str, String)>,
        today: Option<(i32, u8, u8)>,
    ) -> Self {
        let mut dict = Dict::new();
        for (key, value) in inputs {
            dict.insert(key.into(), value.into_value());
        }
        let library = Library::builder().with_inputs(dict).build();
        let fonts = bundled_fonts().to_vec();
        let book = FontBook::from_fonts(&fonts);
        let main = Source::new(file_id(MAIN).expect("a valid path"), source);
        Self {
            library: LazyHash::new(library),
            book: LazyHash::new(book),
            fonts,
            main,
            assets,
            template_dir,
            today: today.and_then(|(y, m, d)| Datetime::from_ymd(y, m, d)),
            read: Mutex::new(HashMap::new()),
        }
    }

    /// The bytes of `id`: a mounted asset or a file in the template directory.
    fn load(&self, id: FileId) -> FileResult<Bytes> {
        let rooted = id.get();
        if let VirtualRoot::Package(spec) = rooted.root() {
            return Err(FileError::Package(PackageError::Other(Some(
                format!("packages are not available in mrq (`{spec}`)").into(),
            ))));
        }
        let path = rooted.vpath().get_with_slash();
        match self.assets.get(path) {
            Some(Asset::Bytes(bytes)) => return Ok(bytes.clone()),
            None => {}
        }
        if let (Some(dir), Some(rest)) = (
            &self.template_dir,
            path.strip_prefix(TEMPLATE_DIR)
                .and_then(|rest| rest.strip_prefix('/')),
        ) {
            return read(&dir.join(rest));
        }
        Err(FileError::NotFound(PathBuf::from(path)))
    }
}

fn read(path: &Path) -> FileResult<Bytes> {
    match std::fs::read(path) {
        Ok(data) => Ok(Bytes::new(data)),
        Err(err) if path.is_dir() => {
            drop(err);
            Err(FileError::IsDirectory)
        }
        Err(err) => Err(FileError::from_io(err, path)),
    }
}

impl World for MrqWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            return Ok(self.main.clone());
        }
        let bytes = self.file(id)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| FileError::InvalidUtf8)?;
        Ok(Source::new(id, text.to_owned()))
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main.id() {
            return Ok(Bytes::from_string(self.main.clone()));
        }
        let mut read = self
            .read
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read.entry(id).or_insert_with(|| self.load(id)).clone()
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        self.today
    }
}
