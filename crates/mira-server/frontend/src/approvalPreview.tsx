import { createRoot } from 'react-dom/client';
import { Composer } from './components/Composer';
import './styles.css';
const noop = () => {};
document.documentElement.dataset.theme = 'light';
createRoot(document.getElementById('root')!).render(<div className="mx-auto max-w-3xl px-8 pt-12"><Composer {...({disabled:false,busy:false,mode:'edit',model:'gpt-6.1-sol',cwd:'/tmp',engine:{kind:'agent',driver:'codex',display_name:'Codex',model:'gpt-6.1-sol',status:'ready'},skills:[],commands:[],agents:[],engines:[],environments:[],onSend:noop,onSetMode:noop,onSetModel:noop,onSetModelOption:noop,onOpenPicker:noop,onCwdSwitched:noop,onInterrupt:noop,onNewChat:noop,onOpenSettings:noop,onDecide:noop,onPlanReply:noop,onAskUserReply:noop} as any)} /></div>);
