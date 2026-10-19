import base64, json, subprocess, urllib.request

tools = [
    ("roc_desk-http", "HTTP \u6d4b\u8bd5\u5de5\u4f5c\u53f0", "\u7528\u4e8e\u521b\u5efa\u3001\u7ba1\u7406\u548c\u6267\u884c HTTP \u8bf7\u6c42\uff0c\u652f\u6301\u96c6\u5408\u3001\u73af\u5883\u53d8\u91cf\u3001\u8bf7\u6c42\u5386\u53f2\u4ee5\u53ca Postman/OpenAPI \u5bfc\u5165\u5bfc\u51fa\u3002", "roc_desk-common core\uff1breqwest\uff1bserde/serde_json/serde_yaml\uff1buuid\uff1bchrono"),
    ("roc_desk-explorer", "\u8d44\u6e90\u7ba1\u7406\u5668", "\u63d0\u4f9b\u672c\u5730\u548c\u8fdc\u7a0b\u6587\u4ef6\u7684\u53cc\u680f\u6d4f\u89c8\u3001\u76ee\u5f55\u6811\u3001\u591a\u9009\u3001\u590d\u5236\u79fb\u52a8\u3001\u91cd\u547d\u540d\u3001\u5220\u9664\u548c\u6587\u4ef6\u9884\u89c8\u3002", "roc_desk-common core/common\uff1bFileOps \u6587\u4ef6\u8bbf\u95ee\u5951\u7ea6\uff1bSSH/SFTP \u6216 Agent \u9002\u914d\u5668"),
    ("roc_desk-editor", "\u72ec\u7acb\u7f16\u8f91\u5668", "\u63d0\u4f9b\u591a\u6807\u7b7e\u6587\u672c\u7f16\u8f91\u3001\u7f16\u7801\u5904\u7406\u3001\u56fe\u7247/PDF/Office/JAR/EXE \u9884\u89c8\u548c\u72ec\u7acb\u6587\u4ef6\u6811\u3002", "roc_desk-common core\uff1bFileOps \u4e0e Symbols \u5951\u7ea6\uff1bMonaco Editor"),
    ("roc_desk-sql", "SQL \u5de5\u4f5c\u53f0", "\u63d0\u4f9b\u6570\u636e\u6e90\u7ba1\u7406\u3001SQL \u7f16\u8f91\u6267\u884c\u3001\u5bf9\u8c61\u6d4f\u89c8\u3001\u8868\u683c\u6570\u636e\u7f16\u8f91\u3001\u5bfc\u5165\u5bfc\u51fa\u548c SQL AI \u52a9\u624b\u3002", "roc_desk-common core\uff1brusqlite/r2d2\uff1bPostgreSQL/MySQL/SQL Server \u9a71\u52a8\uff1bAI \u63a5\u53e3"),
    ("roc_desk-ssh", "SSH/SFTP/Agent/RDP \u5de5\u5177", "\u63d0\u4f9b SSH \u7ec8\u7aef\u3001SFTP \u6587\u4ef6\u4f20\u8f93\u3001Windows Agent \u8fde\u63a5\u548c RDP \u8fdc\u7a0b\u684c\u9762\u80fd\u529b\u3002", "roc_desk-common core\uff1brussh/russh-sftp\uff1bWindows Agent protocol\uff1bWindows RDP \u5916\u58f3"),
    ("roc_desk-workspace", "\u7f16\u7a0b\u5de5\u4f5c\u533a", "\u63d0\u4f9b Coding Agent\u3001PTY \u7ec8\u7aef\u3001Git \u64cd\u4f5c\u3001\u7b26\u53f7\u7d22\u5f15\u3001MCP\u3001Skills \u548c AI \u7f16\u7a0b\u8f85\u52a9\u3002", "roc_desk-common core/common\uff1broc_desk-editor\uff1bPTY/Git/AI Provider \u63a5\u53e3"),
]

credential = subprocess.check_output(["git", "credential", "fill"], input=b"protocol=https\nhost=github.com\n\n").decode()
token = next(line[9:] for line in credential.splitlines() if line.startswith("password="))
headers = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
for name, title, purpose, deps in tools:
    body = f"""# {title}

`{name}` \u662f roc_desk \u591a\u4ed3\u5e93\u62c6\u5206\u540e\u7684\u72ec\u7acb\u5de5\u5177\u4ed3\u5e93\u3002

## \u7528\u9014

{purpose}

## \u4f9d\u8d56

- \u516c\u5171\u57fa\u7840\u5e93\uff1a`roc_desk-common` \u7684 `roc_desk_core`\uff0c\u56fa\u5b9a\u4f7f\u7528\u5bf9\u5e94 tag\u3002
- \u5de5\u5177\u4e13\u5c5e\u4f9d\u8d56\uff1a{deps}\u3002
- \u72ec\u7acb\u58f3\uff1a`standalone` crate\uff0c\u8d1f\u8d23\u751f\u6210\u8be5\u5de5\u5177\u81ea\u5df1\u7684 EXE\u3002

## \u6784\u5efa EXE

```powershell
.\\build-standalone.ps1
```

\u9ed8\u8ba4\u751f\u6210 Release \u7248\u672c\uff1a`bin\\{name}.exe`\u3002\u5f00\u53d1\u6784\u5efa\u4f7f\u7528：

```powershell
.\\build-standalone.ps1 -Configuration debug
```

## \u8fd0\u884c\u622a\u56fe

\u622a\u56fe\u653e\u5728 `docs/screenshots/`\uff0c\u4f8b\u5982：

```markdown
![{title}\u4e3b\u754c\u9762](docs/screenshots/main.png)
```

\u5f53\u524d\u4ed3\u5e93\u5df2\u9884\u7559\u622a\u56fe\u76ee\u5f55\uff0c\u540e\u7eed\u8865\u5145\u5b9e\u9645\u8fd0\u884c\u622a\u56fe\u3002

## \u8fc1\u79fb\u72b6\u6001

\u4e1a\u52a1\u6e90\u7801\u5df2\u7ecf\u4ece\u5bbf\u4e3b\u4ed3\u5e93\u590d\u5236\u5230\u672c\u4ed3\u5e93\u3002\u5bbf\u4e3b\u6682\u65f6\u4fdd\u7559\u517c\u5bb9\u5b9e\u73b0\uff0c\u5b8c\u6210\u516c\u5171\u63a5\u53e3\u9002\u914d\u3001\u72ec\u7acb EXE \u9a8c\u8bc1\u548c\u5bbf\u4e3b Git \u4f9d\u8d56\u5207\u6362\u540e\uff0c\u518d\u5220\u9664\u5bbf\u4e3b\u526f\u672c\u3002

\u603b\u4f53\u65b9\u6848\u89c1：[\u591a\u4ed3\u5e93\u62c6\u5206\u8ba1\u5212](https://github.com/swimhigh/roc_desk/blob/main/docs/MULTI_REPO_SPLIT_PLAN.md)\u3002
"""
    url = f"https://api.github.com/repos/swimhigh/{name}/contents/README.md"
    req = urllib.request.Request(url, headers=headers)
    old = json.load(urllib.request.urlopen(req))
    payload = {"message": "完善中文用途依赖说明和截图占位", "content": base64.b64encode(body.encode()).decode(), "sha": old["sha"], "branch": "main"}
    req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers={**headers, "Content-Type": "application/json"}, method="PUT")
    urllib.request.urlopen(req).read()
    shot_url = f"https://api.github.com/repos/swimhigh/{name}/contents/docs/screenshots/README.md"
    shot = {"message": "预留运行截图目录", "content": base64.b64encode("# 运行截图\n\n将实际运行截图放在这里，并在仓库 README.md 中引用。\n".encode()).decode(), "branch": "main"}
    try:
        urllib.request.urlopen(urllib.request.Request(shot_url, data=json.dumps(shot).encode(), headers={**headers, "Content-Type": "application/json"}, method="PUT")).read()
    except urllib.error.HTTPError as e:
        if e.code != 422:
            raise
    print("UPDATED", name)
