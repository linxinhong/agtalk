// Phase 2 最小占位：仅暴露注入结果类型与少量安全 helper。
// 不迁移旧 content.js 的完整注入逻辑，也不改变选择器行为。

export interface InjectResult {
  success: boolean;
  method?: 'click' | 'enter' | 'manual';
  error?: string;
}

export interface InputElement extends HTMLElement {
  value?: string;
}

export function dispatchInputEvents(el: HTMLElement): void {
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  el.dispatchEvent(new Event('keyup', { bubbles: true }));
}

export function createEnterKeydown(): KeyboardEvent {
  return new KeyboardEvent('keydown', {
    key: 'Enter',
    code: 'Enter',
    keyCode: 13,
    which: 13,
    bubbles: true,
    cancelable: true,
    shiftKey: false,
  });
}

export function createEnterKeyup(): KeyboardEvent {
  return new KeyboardEvent('keyup', {
    key: 'Enter',
    code: 'Enter',
    keyCode: 13,
    which: 13,
    bubbles: true,
    cancelable: true,
    shiftKey: false,
  });
}
