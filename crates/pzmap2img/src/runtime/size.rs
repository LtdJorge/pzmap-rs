/// Reports roughly how much heap memory a value owns.
///
/// The figures are for the build's memory limits and for reporting, so they are close but not
/// exact. Memory held by the tile and room name tables is not counted.
pub trait HeapSize {
    /// Returns the bytes this value owns on the heap, not counting the value itself.
    fn heap_size(&self) -> usize;
}

impl<T: HeapSize> HeapSize for [T] {
    fn heap_size(&self) -> usize {
        self.iter().map(HeapSize::heap_size).sum()
    }
}

impl<T: HeapSize> HeapSize for Option<T> {
    fn heap_size(&self) -> usize {
        self.as_ref().map_or(0, HeapSize::heap_size)
    }
}
