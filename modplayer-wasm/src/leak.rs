use std::os::raw::c_void;

pub(crate) fn leak<O>(obj: &O) -> *mut c_void {
    let on_the_heap = Box::new(obj);
    let leaked_pointer = Box::into_raw(on_the_heap);
    let untyped_pointer = leaked_pointer as *mut c_void;

    untyped_pointer
}

pub(crate) fn leak_mut<O>(obj: &mut O) -> *mut c_void {
    let on_the_heap = Box::new(obj);
    let leaked_pointer = Box::into_raw(on_the_heap);
    let untyped_pointer = leaked_pointer as *mut c_void;

    untyped_pointer
}
