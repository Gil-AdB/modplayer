#[macro_export]
macro_rules! leak {
    ($name:expr_2021) => {
        Box::into_raw(Box::new($name)) as *mut c_void
    }
}
