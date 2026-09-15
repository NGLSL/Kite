fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.set("ProductName", "Kite");
        res.set("FileDescription", "Kite Windows launcher");
        res.set("CompanyName", "Kite Contributors");
        res.compile()
            .expect("failed to embed Windows application icon");
    }
}
