//! Tables that give each distinct tile or room name a small integer id.
//!
//! Storing ids instead of strings keeps the map small, and lets a tile id index straight into
//! the texture library. A table goes through three stages:
//!
//! 1. `StringTableBuilder` adds names from many threads while the lotheaders are parsed.
//! 2. `StringTableReader` looks up names and ids in both directions without locking.
//! 3. [`StringTable`] only turns ids back into names. This is what the finished map keeps.
use crate::error::BuildError;
use lasso::{
    Key,
    RodeoReader,
    RodeoResolver,
    Spur,
    ThreadedRodeo,
};
use std::{
    fmt,
    hash::Hash,
    marker::PhantomData,
};

/// Marks which kind of name a table holds, so ids from different tables can't be mixed up.
///
/// Each table numbers its names from zero with no gaps, so an id can be used as an array
/// index. [`TextureLibrary::metas`](crate::textures::TextureLibrary::metas) is indexed by
/// [`TileId`] this way. Keeping room names in their own table keeps the tile ids dense.
pub trait Namespace: 'static {
    /// A short name for the kind, used in errors and debug output.
    const NAME: &'static str;
}

/// Tile names, from the lotheaders' name tables. A tile name is also the name of the sprite
/// that draws it.
#[derive(Copy, Clone, Debug)]
pub struct TileNames;

/// Room names, from the lotheaders' room lists. Only the annotations use them.
#[derive(Copy, Clone, Debug)]
pub struct RoomNames;

impl Namespace for TileNames {
    const NAME: &'static str = "tile";
}

impl Namespace for RoomNames {
    const NAME: &'static str = "room";
}

/// The id of a name in a [`StringTable`].
///
/// It is 4 bytes, and `Option<Id<S>>` is 4 bytes too.
#[repr(transparent)]
pub struct Id<S: Namespace> {
    raw: Spur,
    _ns: PhantomData<fn() -> S>,
}

impl<S: Namespace> Id<S> {
    /// Returns the id as an index in `0..table.len()`.
    #[inline]
    pub fn index(self) -> usize {
        self.raw.into_usize()
    }

    /// Makes an id from an index. Returns `None` if the index is too large to be an id.
    ///
    /// The id isn't checked against any table.
    #[inline]
    pub fn from_index(index: usize) -> Option<Self> {
        Spur::try_from_usize(index).map(Id::from_spur)
    }

    #[inline]
    fn from_spur(raw: Spur) -> Self {
        Id {
            raw,
            _ns: PhantomData,
        }
    }
}

// Written by hand because derived impls would require the marker type `S` to implement them.
impl<S: Namespace> Copy for Id<S> {}

impl<S: Namespace> Clone for Id<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Namespace> PartialEq for Id<S> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<S: Namespace> Eq for Id<S> {}

impl<S: Namespace> PartialOrd for Id<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<S: Namespace> Ord for Id<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index().cmp(&other.index())
    }
}

impl<S: Namespace> Hash for Id<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<S: Namespace> fmt::Debug for Id<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", S::NAME, self.index())
    }
}

/// The id of a tile name. It is also the index of the tile's sprite in the texture library.
pub type TileId = Id<TileNames>;

/// The id of a room name.
pub type RoomNameId = Id<RoomNames>;

/// The first stage of a table: names can be added from many threads at once.
#[derive(Debug)]
pub(crate) struct StringTableBuilder<S: Namespace> {
    interner: ThreadedRodeo<Spur>,
    _ns: PhantomData<fn() -> S>,
}

impl<S: Namespace> Default for StringTableBuilder<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Namespace> StringTableBuilder<S> {
    /// Creates an empty table.
    pub fn new() -> Self {
        StringTableBuilder {
            interner: ThreadedRodeo::new(),
            _ns: PhantomData,
        }
    }

    /// Returns the id for `string`, adding it if it is new.
    ///
    /// Takes `&self`, so many threads can add names to one table at once.
    pub fn intern(&self, string: impl AsRef<str>) -> Result<Id<S>, BuildError> {
        self.interner
            .try_get_or_intern(string)
            .map(Id::from_spur)
            .map_err(|source| BuildError::Intern {
                namespace: S::NAME,
                source: Box::new(source),
            })
    }

    /// Returns the number of distinct names added so far.
    pub fn len(&self) -> usize {
        self.interner.len()
    }

    /// Stops adding names, giving a table that looks up names and ids without locking.
    pub fn into_reader(self) -> StringTableReader<S> {
        StringTableReader {
            reader: self.interner.into_reader(),
            _ns: PhantomData,
        }
    }

    /// Stops adding names, giving the final id-to-name table.
    pub fn build(self) -> StringTable<S> {
        StringTable {
            resolver: self.interner.into_resolver(),
            _ns: PhantomData,
        }
    }
}

/// The second stage of a table: no more names can be added, and lookups in both directions
/// need no locking.
pub(crate) struct StringTableReader<S: Namespace> {
    reader: RodeoReader<Spur>,
    _ns: PhantomData<fn() -> S>,
}

impl<S: Namespace> StringTableReader<S> {
    /// Returns every `(id, name)` pair.
    pub fn iter(&self) -> impl Iterator<Item = (Id<S>, &str)> {
        self.reader
            .iter()
            .map(|(raw, name)| (Id::from_spur(raw), name))
    }

    /// Returns the number of names.
    pub fn len(&self) -> usize {
        self.reader.len()
    }

    /// Returns the id of `string`, or `None` if the table doesn't have it.
    pub fn resolve_string(&self, string: impl AsRef<str>) -> Option<Id<S>> {
        self.reader.get(string).map(Id::from_spur)
    }

    /// Drops the name-to-id lookup, giving the final id-to-name table.
    pub fn build(self) -> StringTable<S> {
        StringTable {
            resolver: self.reader.into_resolver(),
            _ns: PhantomData,
        }
    }
}

/// A finished table that turns ids back into names.
pub struct StringTable<S: Namespace> {
    resolver: RodeoResolver<Spur>,
    _ns: PhantomData<fn() -> S>,
}

impl<S: Namespace> StringTable<S> {
    /// Builds a table from a list of names. Each distinct name gets the next id, starting at 0.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::Intern`] if there are too many names.
    pub fn from_names<T: AsRef<str>>(
        names: impl IntoIterator<Item = T>,
    ) -> Result<Self, BuildError> {
        let builder = StringTableBuilder::new();
        for name in names {
            builder.intern(name)?;
        }
        Ok(builder.build())
    }

    /// Returns the name for `key`, or `None` if the table has no such id.
    pub fn resolve(&self, key: Id<S>) -> Option<&str> {
        self.resolver.try_resolve(&key.raw)
    }

    /// Returns the number of names.
    pub fn len(&self) -> usize {
        self.resolver.len()
    }

    /// Returns true if the table has no names.
    pub fn is_empty(&self) -> bool {
        self.resolver.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StringTableBuilder,
        TileNames,
    };
    use std::thread;

    #[test]
    fn interning_is_concurrent_and_deduplicating() {
        let builder = StringTableBuilder::<TileNames>::new();

        let keys = thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|thread| {
                    let builder = &builder;
                    scope.spawn(move || {
                        let mut keys = Vec::new();
                        for i in 0..64 {
                            keys.push(builder.intern(format!("tile_{i}")).expect("intern"));
                        }
                        keys.push(builder.intern(format!("unique_{thread}")).expect("intern"));
                        keys
                    })
                })
                .collect();

            handles
                .into_iter()
                .map(|handle| handle.join().expect("no worker should panic"))
                .collect::<Vec<_>>()
        });

        assert_eq!(builder.len(), 72);

        let first = &keys[0][..64];
        for other in &keys[1..] {
            assert_eq!(&other[..64], first);
        }

        let table = builder.build();
        for (i, key) in first.iter().enumerate() {
            assert_eq!(table.resolve(*key), Some(format!("tile_{i}").as_str()));
        }
    }

    #[test]
    fn keys_are_dense_from_zero() {
        let builder = StringTableBuilder::<TileNames>::new();
        let keys: Vec<_> = (0..100)
            .map(|i| builder.intern(format!("n{i}")).expect("intern"))
            .collect();

        let mut indices: Vec<_> = keys.iter().map(|k| k.index()).collect();
        indices.sort_unstable();
        assert_eq!(indices, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn resolving_an_out_of_range_key_is_none_not_a_panic() {
        let builder = StringTableBuilder::<TileNames>::new();
        builder.intern("only").expect("intern");
        let table = builder.build();

        let stray = super::TileId::from_index(999).expect("constructible");
        assert_eq!(table.resolve(stray), None);
    }
}
