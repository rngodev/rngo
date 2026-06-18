pub trait FlattenErr<T, E> {
    fn flatten_err(self) -> Result<T, Vec<E>>;
}

impl<T, E> FlattenErr<T, E> for Result<T, Vec<Vec<E>>> {
    fn flatten_err(self) -> Result<T, Vec<E>> {
        self.map_err(|e| e.into_iter().flatten().collect())
    }
}
