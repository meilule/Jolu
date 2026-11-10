// Windows 发布版不弹控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 卸载钩子（NSIS `installer.nsi` 的 Jolu: 段在删除主程序前调用）：把每个已接线
    // agent 的配置文件还原到接线前的值，然后立即退出，不启动 Tauri。
    // 为什么走参数而不是独立 CLI：安装包里只有主程序一个 exe（tauri bundle 不含
    // 额外 bin），卸载时没有 jolu_cli 可调。
    // 结果以退出码体现（0 全成功 / 1 有失败）——发布版无控制台，stdout 不可见。
    if std::env::args().any(|arg| arg == "--restore-agents") {
        std::process::exit(jolu_lib::restore_agents_headless());
    }
    // 搜索 MCP server（批 5）：agent 的 mcpServers 配置指向本 exe + 此参数，
    // agent 启动时拉起本进程、stdio 说 MCP；stdin 关闭（agent 退出）即随之退出。
    if std::env::args().any(|arg| arg == "--mcp-search") {
        jolu_lib::run_mcp_search();
        return;
    }
    jolu_lib::run()
}
