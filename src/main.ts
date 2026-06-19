import { createApp } from "vue";
import { invoke } from "@tauri-apps/api/core";
import App from "./App.vue";
import ApprovalView from "./views/ApprovalView.vue";
import "./styles/main.css";

async function main() {
  let popupMsgId: string | null = null;
  try {
    popupMsgId = await invoke<string | null>("get_popup_focus");
  } catch {
    popupMsgId = null;
  }
  if (popupMsgId !== null) {
    // 审批弹窗模式：daemon spawn 的 __popup 进程
    createApp(ApprovalView, { msgId: popupMsgId }).mount("#app");
  } else {
    createApp(App).mount("#app");
  }
}

main().catch((e) => {
  const app = document.getElementById("app");
  const text = `启动失败: ${String(e)}`;
  if (app) {
    app.textContent = text;
  } else {
    document.body.textContent = text;
  }
});
