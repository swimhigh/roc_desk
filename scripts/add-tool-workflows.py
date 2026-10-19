import base64, json, subprocess, urllib.request

names = ["roc_desk-http", "roc_desk-explorer", "roc_desk-editor", "roc_desk-sql", "roc_desk-ssh", "roc_desk-workspace"]
cred = subprocess.check_output(["git", "credential", "fill"], input=b"protocol=https\nhost=github.com\n\n").decode()
token = next(x[9:] for x in cred.splitlines() if x.startswith("password="))
headers = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
for name in names:
    workflow = f'''name: Build standalone EXE

on:
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:

permissions:
  contents: write

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build EXE
        shell: pwsh
        run: .\\build-standalone.ps1
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: {name}
          path: bin\\{name}.exe
      - name: Create release
        if: startsWith(github.ref, 'refs/tags/')
        uses: softprops/action-gh-release@v2
        with:
          files: bin\\{name}.exe
'''
    path = ".github/workflows/build-standalone.yml"
    url = f"https://api.github.com/repos/swimhigh/{name}/contents/{path}"
    payload = {"message": "添加独立 EXE 构建与发布 workflow", "content": base64.b64encode(workflow.encode()).decode(), "branch": "main"}
    req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers={**headers, "Content-Type": "application/json"}, method="PUT")
    try:
        urllib.request.urlopen(req).read()
    except urllib.error.HTTPError as e:
        if e.code != 422: raise
        old = json.load(urllib.request.urlopen(urllib.request.Request(url, headers=headers)))
        payload["sha"] = old["sha"]
        req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers={**headers, "Content-Type": "application/json"}, method="PUT")
        urllib.request.urlopen(req).read()
    print("UPDATED", name)
