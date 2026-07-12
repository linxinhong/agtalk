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
