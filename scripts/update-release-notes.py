import json, subprocess, urllib.request
cred = subprocess.check_output(["git", "credential", "fill"], input=b"protocol=https\nhost=github.com\n\n").decode()
token = next(x[9:] for x in cred.splitlines() if x.startswith("password="))
h = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
url = "https://api.github.com/repos/swimhigh/roc_desk-releases/releases/tags/v0.1.0"
r = json.load(urllib.request.urlopen(urllib.request.Request(url, headers=h)))
assets = "\n".join(f"- `{a['name']}` — {a['size']} bytes — {a.get('digest', 'unknown')}" for a in r["assets"])
body = f"""# roc_desk 工具首批人工测试版

本 Release 包含六个工具仓库当前可独立构建的 Windows EXE。当前为迁移阶段构建壳，重点用于验证下载、启动和发布链路。

## 文件与校验值

{assets}

## 构建来源

每个文件由对应仓库的 `build-standalone.ps1` 使用 Release 配置生成。业务功能尚未全部接入独立壳，完整功能迁移完成后会发布正式版本。
"""
payload = json.dumps({"body": body}).encode()
req = urllib.request.Request(r["url"], data=payload, headers={**h, "Content-Type": "application/json"}, method="PATCH")
urllib.request.urlopen(req).read()
print("RELEASE_NOTES_UPDATED")
