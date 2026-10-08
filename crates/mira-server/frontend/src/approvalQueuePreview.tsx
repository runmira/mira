/** Development fixture: fake approval calls; no tools execute. */
import { useEffect, useState, type ComponentProps } from 'react';
import { createRoot } from 'react-dom/client';
import { Composer, type PendingApproval } from './components/Composer';
import './styles.css';
document.documentElement.dataset.theme = 'light';
document.documentElement.classList.remove('dark');
const noop = () => {};
const initial: PendingApproval[] = ['src/App.tsx', 'src/types.ts', 'src/main.ts'].map((path, index) => ({ callId: `read-${index}`, call: { id: `read-${index}`, type: 'function', function: { name: 'read_file', arguments: JSON.stringify({path}) } }, preview: null, startedAt: Date.now() - 22000, rulePreview: {rules: [`Read(${path})`], error: null} }));
initial.push({callId:'bash',call:{id:'bash',type:'function',function:{name:'bash',arguments:JSON.stringify({command:'cargo test'})}},preview:null,startedAt:Date.now()-9000,rulePreview:{rules:['Bash(cargo test)'],error:null}});
function Preview() {
  const [theme, setTheme] = useState<'light' | 'dark'>('light');
  useEffect(() => { const root = document.documentElement; root.dataset.theme = theme; root.classList.toggle('dark', theme === 'dark'); root.style.colorScheme = theme; }, [theme]);
  const [pending, setPending] = useState(initial);
  const [result, setResult] = useState('');
  const [focused, setFocused] = useState<string | null>(null);
  const decide = (id: string, allow: boolean, scope?: string, rules?: string[]) => { setResult(`${allow ? 'Allowed' : 'Denied'} ${id} ${scope ?? 'once'} ${rules?.join(', ') ?? ''}`); setPending(previous => previous.filter(item => item.callId !== id)); };
  return <div className="mx-auto max-w-3xl pt-12"><div className="mb-4 flex justify-end gap-2 px-6"><button className="rounded-full border border-border px-4 py-2 text-sm" onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>Switch to {theme === 'light' ? 'dark' : 'light'} mode</button><button className="rounded-full border border-border px-4 py-2 text-sm" onClick={() => {setPending(initial); setResult('');}}>Reset approvals</button></div><p className="px-6 text-sm">Preview · focused: {focused} · {result}</p><Composer {...({disabled:false,busy:false,usage:null,mode:'manual',model:'gpt-6.1-sol',cwd:'/tmp',engine:{kind:'provider',instance:'openai',display_name:'OpenAI',model:'gpt-6.1-sol',status:'ready'},skills:[],commands:[],agents:[],engines:[],environments:[],onSend:noop,onSetMode:noop,onSetModel:noop,onSetModelOption:noop,onOpenPicker:noop,onCwdSwitched:noop,onInterrupt:noop,onNewChat:noop,onOpenSettings:noop,onRunReview:noop,onSetGoal:noop,onClearGoal:noop,onCompact:noop,goal:null,onRemember:async()=>'',onUndo:async()=>'',pendingApprovals:pending,pendingApprovalCount:pending.length,onDecide:decide,onActiveApprovalChange:setFocused,onPlanReply:noop,onAskUserReply:noop} as ComponentProps<typeof Composer>)} /></div>;
}
createRoot(document.getElementById('root')!).render(<Preview />);
