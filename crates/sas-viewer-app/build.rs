fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.ico");
    println!("cargo:rerun-if-changed=assets/app_icon.png");
    println!("cargo:rerun-if-changed=assets/wechat_account.png");

    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app_icon.ico")
            .set("ProductName", "SAS Dataset Viewer")
            .set("FileDescription", "SAS 数据集查看器 (XPT / SAS7BDAT)")
            .set("LegalCopyright", "MIT License - allensrj");
        if let Err(err) = res.compile() {
            println!("cargo:warning=Failed to embed Windows application icon: {err}");
        }
    }
}
