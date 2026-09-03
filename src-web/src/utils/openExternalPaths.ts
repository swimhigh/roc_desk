import { useWorkspaceStore } from "../stores/workspaceStore";
import { useEditorStore } from "../stores/editorStore";
import { localFsService } from "../services/localFsService";
import { useToastStore } from "../components/shared/Toast";
import { formatError } from "./error";

/** 拖拽/Ctrl+O/系统文件关联/首页"打开文件"按钮打开外部路径的统一入口（2026-09-03
 * 需求：像 VSCode/Notepad 一样支持不建工作区直接打开单个文件）。文件夹当工作区
 * 打开，文件当游离标签打开。混合多选时只取第一个文件夹当工作区、其余路径忽略
 * 并提示——这个场景本来就少见，不值得为它设计更复杂的规则。
 *
 * 用 `getState()` 直接操作 store 而不是接收各 store 的 action 作为参数，是因为
 * 调用方（App.tsx 的拖拽/Ctrl+O/文件关联、WorkspacePicker.tsx 首页按钮）分散在
 * 不同组件树里，没有共同的父组件方便传参，且这个函数本身不依赖任何 React 状态。 */
export async function openExternalPaths(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  const push = useToastStore.getState().push;
  const dirFlags = await Promise.all(paths.map((p) => localFsService.isDir(p).catch(() => false)));
  const dirPath = paths.find((_, i) => dirFlags[i]);
  const filePaths = paths.filter((_, i) => !dirFlags[i]);

  if (dirPath) {
    if (paths.length > 1) push("info", `检测到文件夹 ${dirPath}，将作为工作区打开，其余项已忽略`);
    try {
      await useWorkspaceStore.getState().openLocalPath(dirPath);
    } catch (e) {
      push("error", `打开文件夹失败：${formatError(e)}`);
    }
    return;
  }

  let opened = 0;
  for (const path of filePaths) {
    try {
      await useEditorStore.getState().openStandaloneFile(path);
      opened++;
    } catch (e) {
      push("error", `打开文件失败：${formatError(e)}`);
    }
  }
  if (opened === 0) return;
  // 打开成功后要让用户实际看到这个文件，不能只是悄悄进了 store：
  // - 没有工作区（本来就没打开过，或者打开的是游离文件而不是文件夹）——把极简
  //   编辑器壳显示出来；
  // - 有工作区但用户正停在首页（HomeShell.showPicker）——切回工作区视图，
  //   否则新开的标签会被后台隐藏的 IDE 子树吞掉，用户什么反馈都看不到。
  const { current, returnToCurrentWorkspace } = useWorkspaceStore.getState();
  if (!current) {
    useEditorStore.getState().showStandaloneShell();
  } else {
    returnToCurrentWorkspace();
  }
}
