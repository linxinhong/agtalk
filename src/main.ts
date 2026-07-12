import { createApp } from 'vue'
import App from './App.vue'
import i18n from './i18n'
import './styles/tokens.css'
import './styles/base.css'
import './styles/controls.css'

createApp(App).use(i18n).mount('#app')
