
pub fn setup() {
    // setup code specific to your library's tests would go here

}

#[test]
fn basic() {
    let mut song = Song::new(xm_reader::read_xm("../Revival.XM"), 44_100);

    //
    // assert!(song.)
}