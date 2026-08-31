roc_desk_agent 使用说明（远程 Windows Agent）

一、这是什么
roc_desk_agent.exe 不是给你自己这台机器用的——它是部署在【被管理的远程 Windows 服务器】
上的常驻程序，让 roc_desk.exe 能通过"Agent"这种连接方式访问该服务器，不需要那台服务器
装 OpenSSH Server。把 roc_desk_agent.exe 这一个文件拷到目标 Windows 服务器上，其余文件
（roc_desk.exe 等）留在你自己的电脑上就行。

二、快速上手（在目标服务器上，用命令行或 PowerShell）
    .\roc_desk_agent.exe pair                 # 生成配对令牌，只展示这一次
    .\roc_desk_agent.exe run                  # 前台运行（Ctrl+C 退出），测试用
把 pair 打印出来的令牌通过一条可信信道（内部 IM、密码管理器分享）交给需要连接的 roc_desk
用户，对方在 roc_desk.exe 里新建连接时协议选"Agent"，粘贴这个令牌即可。

生产部署（开机自启、常驻后台，不占着一个终端窗口，需要管理员权限）：
    .\roc_desk_agent.exe install-service
    net start RocDeskAgent
卸载：
    net stop RocDeskAgent
    .\roc_desk_agent.exe uninstall-service
如果客户端连不上，多半是 Windows 防火墙拦了入站连接——放行 Agent 监听端口（默认 7879）：
    New-NetFirewallRule -DisplayName roc_desk_agent -Direction Inbound -Protocol TCP -LocalPort 7879 -Action Allow

重新生成令牌（旧令牌立即失效，所有已连接的客户端都要重新填新令牌）：
    .\roc_desk_agent.exe pair

查看当前监听地址、配对状态：
    .\roc_desk_agent.exe status

三、首次运行会在同目录生成
agent.toml   配置文件（监听端口、路径访问范围 allowed_roots、配对令牌哈希）——可以手动编辑，
             比如把 allowed_roots 改成 ["D:\\projects"] 限制只能访问指定目录，而不是整机。
cert.pem / key.pem   自签名 TLS 证书，固定复用；roc_desk 客户端首次连接会弹窗确认证书指纹，
             这是正常的 TOFU（Trust On First Use）流程，和 SSH 首次连接确认主机指纹是一回事。
logs\        运行日志。

四、安全提示
- 只在内网或已建立 VPN/跳板的场景下使用，不要把这个端口直接暴露到公网。
- allowed_roots 留空表示不限制访问范围（等价于"连上就能读写这个账户权限范围内的任何文件"），
  按需自行收紧。
- 配对令牌不会以明文形式存进 agent.toml，只存哈希；丢失/泄露了直接重新 pair 即可让旧令牌失效。

五、系统兼容性
这份 roc_desk_agent.exe 编译时特意做了向下兼容处理，Windows 7 SP1 / Server 2008 R2 及以上
（含 Server 2012、2012 R2）都能运行，不需要额外安装运行库。
