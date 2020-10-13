use crate::leak;

use std::os::raw::{c_int, c_void, c_char};

#[allow(non_camel_case_types)]
type em_callback_func = unsafe extern "C" fn(context: *mut c_void);

struct Env {
    func:   *mut c_void,
    arg:    *mut c_void,
}

impl Env {
    fn new(func: *mut c_void, arg: *mut c_void) -> *mut c_void {
        leak!(Env {
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
    let leaked_func = leak!(callback);
    let leaked_arg = leak!(arg);
    let leaked_env = Env::new(leaked_func, leaked_arg);

    unsafe {
        emscripten_set_main_loop_arg(wrapper::<F>, leaked_env, fps, simulate_infinite_loop)
    }

    extern "C" fn wrapper<F: FnMut(*mut c_void) + 'static>(untyped_pointer: *mut c_void) {

        let leaked_pointer = untyped_pointer as *mut Env;
        let capture = unsafe { &mut *leaked_pointer };

        let mut leaked_pointer_f = capture.func as *mut F;
        let f = unsafe { &mut *leaked_pointer_f };

        f(capture.arg)
    }
}
