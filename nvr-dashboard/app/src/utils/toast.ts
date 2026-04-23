import type { ToastMessageOptions } from 'primevue/toast'
import { useToast } from 'primevue/usetoast'

type ToastSeverity = NonNullable<ToastMessageOptions['severity']>
type ToastDetail = ToastMessageOptions['detail']

const DEFAULT_LIFE: Record<Extract<ToastSeverity, 'success' | 'info' | 'warn' | 'error'>, number> = {
  success: 2000,
  info: 1500,
  warn: 2000,
  error: 2500,
}

export function toErrorMessage(error: unknown, fallback: string) {
  if (error instanceof Error && error.message) {
    return error.message
  }
  return fallback
}

export function useAppToast() {
  const toast = useToast()

  function show(severity: ToastSeverity, summary: string, detail?: ToastDetail, life?: number) {
    toast.add({
      severity,
      summary,
      detail,
      life: life ?? DEFAULT_LIFE[severity as keyof typeof DEFAULT_LIFE],
    })
  }

  return {
    add: toast.add,
    success: (summary: string, detail?: ToastDetail, life?: number) =>
      show('success', summary, detail, life),
    info: (summary: string, detail?: ToastDetail, life?: number) => show('info', summary, detail, life),
    warn: (summary: string, detail?: ToastDetail, life?: number) => show('warn', summary, detail, life),
    error: (summary: string, detail?: ToastDetail, life?: number) => show('error', summary, detail, life),
    errorFrom: (summary: string, error: unknown, fallback: string, life?: number) =>
      show('error', summary, toErrorMessage(error, fallback), life),
  }
}
