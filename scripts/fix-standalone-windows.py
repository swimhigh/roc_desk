from pathlib import Path
import subprocess

root = Path(r"F:\code\wuyou\roc_tools")
tools = [
    ("roc_desk-http", "HTTP 测试工作台"),
    ("roc_desk-explorer", "资源管理器"),
    ("roc_desk-editor", "独立编辑器"),
    ("roc_desk-sql", "SQL 工作台"),
    ("roc_desk-ssh", "SSH/SFTP/Agent/RDP 工具"),
    ("roc_desk-workspace", "编程工作区"),
]
for name, title in tools:
    d = root / name
    cargo = d / "standalone/Cargo.toml"
    text = cargo.read_text(encoding="utf-8")
    if "windows =" not in text:
        text += '\n[target."cfg(windows)".dependencies]\nwindows = { version = "0.61", features = ["Win32_UI_WindowsAndMessaging", "Win32_Foundation"] }\n'
        cargo.write_text(text, encoding="utf-8")
    main = d / "standalone/src/main.rs"
    main.write_text(f'''#[cfg(windows)]
fn main() {{
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{{MessageBoxW, MB_ICONINFORMATION, MB_OK}};
    unsafe {{
        MessageBoxW(None, w!("{title} 独立壳已启动。业务模块正在迁移中。"), w!("{name}"), MB_OK | MB_ICONINFORMATION);
    }}
}}

#[cfg(not(windows))]
fn main() {{
    println!("{name} standalone shell");
    std::thread::park();
}}
''', encoding="utf-8")
    subprocess.run(["git", "-C", str(d), "add", "standalone/Cargo.toml", "standalone/src/main.rs"], check=True)
    subprocess.run(["git", "-C", str(d), "commit", "-m", "修复独立壳启动窗口不立即退出"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "-C", str(d), "push", "origin", "main"], check=True)
    print("UPDATED", name)
