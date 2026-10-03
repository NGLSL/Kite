fn main() {
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("windows"),
        "Kite supports Windows targets only"
    );
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.set("ProductName", "Kite");
        res.set("FileDescription", "Kite");
        res.set("CompanyName", "Kite Contributors");
        res.compile()
            .expect("failed to embed Windows application icon");
    }
}
