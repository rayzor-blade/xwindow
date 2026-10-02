//! Integer handles for objects that live in Rust.
//!
//! Layout: kind in bits 26..30, generation in 19..25, slot index in 0..18.
//! Zero is never a handle. The generation makes a closed handle miss rather
//! than reach whatever took its slot; the kind stops a monitor being read as
//! a window. The layout is private to xwindow.

const INDEX_BITS: i32 = 19;
const GEN_BITS: i32 = 7;
const INDEX_MASK: i32 = (1 << INDEX_BITS) - 1;
const GEN_MASK: i32 = (1 << GEN_BITS) - 1;

/// What a handle names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum Kind {
    Window = 1,
    Monitor = 2,
}

/// Live objects of one kind.
pub struct Slab<T> {
    kind: Kind,
    objects: Vec<Option<T>>,
    generations: Vec<i32>,
    free: Vec<usize>,
}

impl<T> Slab<T> {
    pub const fn new(kind: Kind) -> Self {
        Slab {
            kind,
            objects: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
        }
    }

    /// Store `value` under a new handle; 0 when this kind is full.
    pub fn put(&mut self, value: T) -> i32 {
        let index = match self.free.pop() {
            Some(index) => {
                self.generations[index] = (self.generations[index] + 1) & GEN_MASK;
                index
            }
            None => {
                let index = self.objects.len();
                if index as i32 > INDEX_MASK {
                    return 0;
                }
                self.objects.push(None);
                self.generations.push(0);
                index
            }
        };
        self.objects[index] = Some(value);
        self.handle(index)
    }

    pub fn get(&self, handle: i32) -> Option<&T> {
        self.objects.get(self.slot(handle)?)?.as_ref()
    }

    pub fn get_mut(&mut self, handle: i32) -> Option<&mut T> {
        let slot = self.slot(handle)?;
        self.objects.get_mut(slot)?.as_mut()
    }

    /// Take the object out; its handle names nothing after.
    pub fn remove(&mut self, handle: i32) -> Option<T> {
        let slot = self.slot(handle)?;
        let value = self.objects[slot].take()?;
        self.free.push(slot);
        Some(value)
    }

    /// Every live object, with its handle, oldest slot first.
    pub fn iter(&self) -> impl Iterator<Item = (i32, &T)> {
        self.objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| Some((self.handle(i), o.as_ref()?)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (i32, &mut T)> {
        let kind = self.kind as i32;
        let generations = &self.generations;
        self.objects
            .iter_mut()
            .enumerate()
            .filter_map(move |(i, o)| {
                let handle =
                    (kind << (INDEX_BITS + GEN_BITS)) | (generations[i] << INDEX_BITS) | i as i32;
                Some((handle, o.as_mut()?))
            })
    }

    fn handle(&self, index: usize) -> i32 {
        ((self.kind as i32) << (INDEX_BITS + GEN_BITS))
            | (self.generations[index] << INDEX_BITS)
            | index as i32
    }

    fn slot(&self, handle: i32) -> Option<usize> {
        if handle <= 0 || handle >> (INDEX_BITS + GEN_BITS) != self.kind as i32 {
            return None;
        }
        let index = (handle & INDEX_MASK) as usize;
        let generation = (handle >> INDEX_BITS) & GEN_MASK;
        (self.generations.get(index) == Some(&generation)).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_handle_misses_what_reuses_its_slot() {
        let mut windows = Slab::new(Kind::Window);
        let first = windows.put("a");
        assert_eq!(windows.remove(first), Some("a"));
        let second = windows.put("b");
        assert_ne!(first, second);
        assert_eq!(windows.get(first), None);
        assert_eq!(windows.get(second), Some(&"b"));
        assert_eq!(windows.iter().map(|(h, _)| h).collect::<Vec<_>>(), [second]);
        let monitors: Slab<&str> = Slab::new(Kind::Monitor);
        assert_eq!(monitors.get(second), None);
        assert_eq!(windows.get(0), None);
    }
}
