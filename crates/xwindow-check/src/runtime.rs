//! Carriers with the shapes Rayzor's have: pointer-sized text, bytes and
//! futures, and integer enums. They leak, and raise by printing and keeping
//! the message for `host::raised`.

use std::marker::PhantomData;

#[derive(Clone, Copy, Debug)]
pub enum ErrorKind {
    Type,
    Runtime,
}

pub mod host {
    use super::ErrorKind;
    use std::cell::RefCell;

    thread_local! {
        static RAISED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    pub fn raise(kind: ErrorKind, message: &str) {
        eprintln!("{kind:?}: {message}");
        RAISED.with(|r| r.borrow_mut().push(format!("{kind:?}: {message}")));
    }

    /// What this thread raised since the last call.
    pub fn raised() -> Vec<String> {
        RAISED.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    pub fn agent(_: &str, _: usize) -> bool {
        false
    }
}

pub trait NativeEnum: Copy + Default {
    fn native(self) -> i32;
    fn from_native(value: i32) -> Option<Self>;
}

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Enum<T: NativeEnum>(i64, PhantomData<fn() -> T>);

impl<T: NativeEnum> Enum<T> {
    pub fn get(self) -> T {
        T::from_native(self.0 as i32).unwrap_or_default()
    }
}

impl<T: NativeEnum> From<T> for Enum<T> {
    fn from(value: T) -> Self {
        Self(i64::from(value.native()), PhantomData)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Text(*mut String);

impl Text {
    pub const NULL: Self = Self(std::ptr::null_mut());

    pub fn new(value: &str) -> Self {
        Self(Box::into_raw(Box::new(value.to_owned())))
    }

    pub fn as_str(&self) -> &str {
        if self.0.is_null() {
            ""
        } else {
            unsafe { &*self.0 }
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Buffer(*mut Vec<u8>);

impl Buffer {
    pub const NULL: Self = Self(std::ptr::null_mut());

    pub fn new(bytes: &[u8]) -> Self {
        Self(Box::into_raw(Box::new(bytes.to_vec())))
    }

    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.0.is_null() {
            &[]
        } else {
            unsafe { &*self.0 }
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct BufferMut(Buffer);

#[repr(transparent)]
pub struct Future<T = ()>(*mut u8, PhantomData<fn() -> T>);

impl<T> Future<T> {
    pub const NULL: Self = Self(std::ptr::null_mut(), PhantomData);
}

#[derive(Clone)]
pub struct Rooted<T: Clone>(T);

impl<T: Clone> Rooted<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn get(&self) -> T {
        self.0.clone()
    }
}
