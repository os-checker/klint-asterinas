use std::any::{Any, TypeId};
use std::collections::hash_map as map;
use std::marker::{PhantomData, Unsize};

use rustc_data_structures::fx::FxHashMap;

/// Map that can store data for arbitrary types.
pub struct AnyMap<U: ?Sized> {
    // This is basically `FxHashMap<TypeId, Box<dyn Any>>`
    //
    // The generic `U` is present to capture auto trait bounds.
    map: FxHashMap<TypeId, Box<U>>,
}

pub struct OccupiedEntry<'a, U: ?Sized, T> {
    entry: map::OccupiedEntry<'a, TypeId, Box<U>>,
    phantom: PhantomData<T>,
}

impl<'a, U: Any + ?Sized + 'static, T: 'static> OccupiedEntry<'a, U, T> {
    pub fn into_mut(self) -> &'a mut T
    where
        T: Unsize<U>,
    {
        let any_ref = &mut **self.entry.into_mut();
        debug_assert_eq!((*any_ref).type_id(), TypeId::of::<T>());
        // SAFETY: by type invariant, `any_ref` is a `&mut T`.
        unsafe { &mut *(any_ref as *mut U as *mut T) }
    }
}

pub struct VacantEntry<'a, U: ?Sized, T> {
    entry: map::VacantEntry<'a, TypeId, Box<U>>,
    phantom: PhantomData<T>,
}

impl<'a, U: Any + ?Sized, T> VacantEntry<'a, U, T> {
    pub fn insert(self, value: T) -> &'a mut T
    where
        T: Unsize<U>,
    {
        let any_ref = &mut **self.entry.insert(Box::new(value) as _);
        // SAFETY: we just inserted it and we know the type is `Box<T>`.
        unsafe { &mut *(any_ref as *mut U as *mut T) }
    }
}

pub enum Entry<'a, U: ?Sized, T> {
    Occupied(OccupiedEntry<'a, U, T>),
    Vacant(VacantEntry<'a, U, T>),
}

impl<'a, U: Any + ?Sized, T: 'static> Entry<'a, U, T> {
    pub fn or_insert_with<F: FnOnce() -> T>(self, default: F) -> &'a mut T
    where
        T: Unsize<U>,
    {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default()),
        }
    }
}

impl<U: ?Sized> Default for AnyMap<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U: ?Sized> AnyMap<U> {
    pub fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }
}

impl<U: Any + ?Sized> AnyMap<U> {
    pub fn entry<T: 'static>(&mut self) -> Entry<'_, U, T> {
        match self.map.entry(TypeId::of::<T>()) {
            map::Entry::Occupied(entry) => Entry::Occupied(OccupiedEntry {
                entry,
                phantom: PhantomData,
            }),
            map::Entry::Vacant(entry) => Entry::Vacant(VacantEntry {
                entry,
                phantom: PhantomData,
            }),
        }
    }
}
