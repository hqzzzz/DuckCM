//! 发布版 Windows 资源：主程序图标（blur.ico）+ 版本信息

fn main() {
    println!("cargo:rerun-if-changed=resources/blur.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("resources/blur.ico");
        res.set("ProductName", "DuckCM 2 · 养殖用房屋设备自动化计算系统");
        res.set("FileDescription", "DuckCM 2");
        res.set("LegalCopyright", "青岛鑫光正钢结构股份有限公司");
        if let Err(e) = res.compile() {
            println!("cargo:warning=图标资源嵌入失败（窗口图标仍生效）: {e}");
        }
    }
}
