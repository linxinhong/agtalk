import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

export interface PopupMessage {
  id: string
  to_address: string
  to_name: string
  from_address: string
  from_name: string
  body: string
  content_type: string
  reply_to_id: string | null
  subject: string | null
  metadata: string
  event_id: number
  status: string
  created_at: number
}

export interface PopupView {
  message: PopupMessage
  human_address: string
}

export const popupLoad = () => invoke<PopupView>('popup_load')

export const popupReply = (body: string, choice?: string) =>
  invoke<string>('popup_reply', { body, choice: choice ?? null })

export const popupDone = () => invoke<void>('popup_done')

/** Later / 操作完成后关窗；直接关窗不改变消息状态（dismissed）。 */
export const closePopup = () => getCurrentWindow().close()

export interface GuiConfigView {
  config: Record<string, unknown>
  path: string
}

export const guiLoadConfig = () => invoke<GuiConfigView>('gui_load_config')

export const guiSetConfig = (key: string, value: string) =>
  invoke<void>('gui_set_config', { key, value })

/** 飞书一键创建应用：设备授权流 begin 结果。 */
export interface FeishuSetupBegin {
  url: string
  device_code: string
  interval_secs: number
  expires_in: number
}

/** 飞书一键创建应用：poll 结果（serde tag = status）。 */
export type FeishuSetupPoll =
  | { status: 'pending' }
  | { status: 'slow_down'; interval_secs: number }
  | {
      status: 'success'
      app_id: string
      app_secret: string
      open_id: string
      tenant_brand: string | null
    }
  | { status: 'denied' }
  | { status: 'expired' }

export const guiFeishuSetupBegin = () =>
  invoke<FeishuSetupBegin>('gui_feishu_setup_begin')

export const guiFeishuSetupPoll = (deviceCode: string) =>
  invoke<FeishuSetupPoll>('gui_feishu_setup_poll', { deviceCode })
