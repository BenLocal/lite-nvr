import 'primeicons/primeicons.css'
import { createApp } from 'vue'
import PrimeVue from 'primevue/config'
import Aura from '@primeuix/themes/aura'
import ToastService from 'primevue/toastservice'
import ConfirmationService from 'primevue/confirmationservice'
import App from './App.vue'
import router from './router'

const app = createApp(App)

// 全局控件小一号：通过 Pass Through 为各组件根节点添加 small 的 class
app.use(PrimeVue, {
  theme: { preset: Aura },
  ptOptions: { mergeSections: true, mergeProps: true },
  pt: {
    Button: { root: { class: 'p-button-sm' } },
    InputText: { root: { class: 'p-inputtext-sm p-inputfield-sm' } },
    Password: { root: { class: 'p-inputfield-sm' }, pcInputText: { class: 'p-inputtext-sm p-inputfield-sm' } },
    Checkbox: { root: { class: 'p-checkbox-sm p-inputfield-sm' } },
    Message: { root: { class: 'p-message-sm' } },
    DataTable: { root: { class: 'p-datatable-sm' } },
  },
})
app.use(ToastService)
app.use(ConfirmationService)
app.use(router)

app.mount('#app')
