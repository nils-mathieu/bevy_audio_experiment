pub struct Discrete<T> {
    // TODO: Replace with a structure-of-array in a single allocation.
    data: Vec<(u32, T)>,
}

impl<T> Discrete<T> {
    pub fn with_capacity(capacity: usize, initial_value: T) -> Self {
        let mut data = Vec::with_capacity(capacity.checked_add(1).expect("Capacity overflow"));
        data.push((0, initial_value));
        Self { data }
    }

    pub fn last(&self) -> (u32, &T) {
        // SAFETY: `data` always contains at least one value.
        let (timestamp, val) = unsafe { self.data.last().unwrap_unchecked() };
        (*timestamp, val)
    }

    pub fn last_timestamp(&self) -> u32 {
        self.last().0
    }

    pub fn insert(&mut self, timestamp: u32, item: T) {
        // Fast path: `timestamp > last_timestamp`
        if timestamp >= self.last_timestamp() {
            self.data.push((timestamp, item));
            return;
        }

        match self.data.binary_search_by_key(&timestamp, |(ts, _)| *ts) {
            // SAFETY: The binary search returns valid indices.
            Ok(idx) => unsafe { *self.data.get_unchecked_mut(idx) = (timestamp, item) },
            Err(idx) => self.data.insert(idx, (timestamp, item)),
        }
    }
}

impl<T> Default for Discrete<T> {
    fn default() -> Self {
        Self { data: Vec::new() }
    }
}
