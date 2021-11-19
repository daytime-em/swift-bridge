use swift_bridge::string::SwiftString;

// TODO: Expose an extern Rust function that creates a String
//  (somewhere along the way it becomes a swift_bridge::string::RustString)
//  Then in the Swift string tests file in Xcode call all of the methods on the RustString

#[no_mangle]
pub extern "C" fn run_string_tests() {
    let string = SwiftString::new_with_str("hello");

    assert_eq!(string.len(), 5);

    assert_eq!(string.to_str(), "hello");
}
