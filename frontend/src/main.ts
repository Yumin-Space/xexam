// 应用入口：把状态机的 view 映射到页面，注册重绘函数，启动。
import './styles/main.css';
import { brand } from './components/common.ts';
import { renderAdmin } from './pages/admin.ts';
import { renderHome } from './pages/home.ts';
import { installQuizKeys, renderQuiz } from './pages/quiz.ts';
import { renderResult } from './pages/result.ts';
import { boot, setRenderer } from './state/actions.ts';
import { S } from './state/store.ts';
import { h, must } from './utils/dom.ts';

function view(): HTMLElement {
  switch (S.view) {
    case 'loading':
      return h('div', { class: 'wrap' }, h('p', { class: 'dim' }, '正在读取题库…'));
    case 'fatal':
      return h('div', { class: 'wrap' }, h('header', { class: 'top' }, brand()),
        h('div', { class: 'err' }, h('p', null, '连不上服务。请确认程序（或 Docker 容器）还在运行，然后刷新这个页面。如果你是通过域名访问的，也可能是登录已过期，刷新后重新登录即可。')));
    case 'admin':
      return renderAdmin();
    case 'quiz':
      if (S.session) return renderQuiz();
      break;
    case 'result':
      if (S.result) return renderResult();
      break;
    default:
      break;
  }
  S.view = 'home';
  return renderHome();
}

function render(): void {
  if (S.timer != null) window.clearInterval(S.timer);
  S.timer = null;
  const el = view();
  must<HTMLElement>('#app').replaceChildren(el);
  document.title = S.view === 'quiz' && S.session ? `第 ${S.session.idx + 1} / ${S.session.qs.length} 题 - Xexam` : 'Xexam';
}

setRenderer(render);
installQuizKeys();
void boot();
