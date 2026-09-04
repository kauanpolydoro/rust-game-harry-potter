/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'

  const component: DefineComponent
  export default component
}

interface Window {
  __HOGWARTS_RECOVERY_TOKEN__?: string | null
}
