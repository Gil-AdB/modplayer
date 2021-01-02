#[macro_export]
macro_rules! leak {
    ($name:expr) => {
        Box::into_raw(Box::new($name)) as *mut c_void
    }
}
