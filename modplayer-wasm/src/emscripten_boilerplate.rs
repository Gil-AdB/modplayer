use std::os::raw::{c_int, c_void, c_char};
use crate::leak::leak;

#[allow(non_camel_case_types)]
type em_callback_func = unsafe extern "C" fn(context: *mut c_void);

struct Capture {
    func:   *mut c_void,
    arg:    *mut c_void,
}

impl Capture {
    fn new(func: *mut c_void, arg: *mut c_void) -> *mut c_void {
        leak(&Capture {
            func,
            arg
        })
    }
}

extern "C" {
    pub fn emscripten_set_main_loop_arg(
        func: em_callback_func,
        arg: *mut c_void,
        fps: c_int,
        simulate_infinite_loop: c_int,
    );

    pub fn emscripten_cancel_main_loop();

    pub fn emscripten_run_script(code: *const c_char);
}

pub fn setup_mainloop<A, F: FnMut(*mut c_void) + 'static>(
    fps: c_int,
    simulate_infinite_loop: c_int,
    arg: A,
    callback: F,
) {
    // let on_the_heap_f = Box::new(callback);
    // let leaked_pointer_f = Box::into_raw(on_the_heap_f);
    // let untyped_pointer_f = leaked_pointer_f as *mut c_void;
    let untyped_pointer_f = leak(&callback);

    // let on_the_heap_a = Box::new(arg);
    // let leaked_pointer_a = Box::into_raw(on_the_heap_a);
    // let untyped_pointer_a = leaked_pointer_a as *mut c_void;
    let untyped_pointer_a = leak(&arg);

    let untyped_pointer_c = Capture::new(untyped_pointer_f, untyped_pointer_a);

    unsafe {
        emscripten_set_main_loop_arg(wrapper::<F>, untyped_pointer_c, fps, simulate_infinite_loop)
    }

    extern "C" fn wrapper<F: FnMut(*mut c_void) + 'static>(untyped_pointer: *mut c_void) {

        let leaked_pointer = untyped_pointer as *mut Capture;
        let capture = unsafe { &mut *leaked_pointer };

        let mut leaked_pointer_f = capture.func as *mut F;
        let f = unsafe { &mut *leaked_pointer_f };

        f(capture.arg)
    }
}
