import {useState} from 'react';
import {Keyboard, Pencil, Plus, RotateCcw, Search, Trash2, X} from 'lucide-react';
import {DEFAULT_KEYBINDINGS, MIRA_KEYBINDING_COMMANDS, parseKeybindingShortcut, shortcutConflictKey, formatShortcutLabel, resetKeybindingCommand, setCommandKeybindings, useKeybindings, type MiraKeybindingCommand, type KeybindingRule} from '../../lib/keybindings';
import {commandLabel, keybindingFromKeyboardEvent, shortcutToKeybindingInput, whenAstToExpression} from './keybindingsLogic';

const descriptions: Partial<Record<MiraKeybindingCommand,string>>={
 'panel.tests':'Open the test runner and inspect test results.',
 'panel.activity':'Browse the current chat’s activity and history.',
 'panel.devices':'Preview your app at different device sizes.',
 'panel.whiteboard':'Sketch ideas in the side panel.',
 'panel.aside':'Open a side question without interrupting your chat.',
 'panel.devtools':'Inspect the app in the developer tools panel.',
 'panel.next':'Move to the next open side panel tab.',
 'panel.previous':'Move to the previous open side panel tab.',
 'panel.closeTab':'Close the selected side panel tab.',
 'chat.bottom':'Scroll to the latest message in this chat.',
 'chat.top':'Scroll to the beginning of the loaded conversation.',
 'composer.attach':'Choose images or files to attach to your next message.',

 'chat.new':'Start a new chat with your current provider or external agent.',
 'composer.focus':'Move the cursor to your message composer.',
 'chat.stop':'Interrupt the current response in this chat.',
 'chat.copyResponse':'Copy the most recent assistant response.',
 'chat.copyCode':'Copy the most recent assistant code block.',
 'panel.files':'Browse project files and open them in the side panel.',
 'panel.processes':'View background commands, logs, and stop controls.',
 'panel.browser':'Open the browser in the side panel.',
 'panel.close':'Close the right side panel.',
 'terminal.toggle':'Show or hide the terminal.',
 'sidebar.toggle':'Show or hide your chats and projects.',
 'review.toggle':'Show or hide the changes review.',
 'palette.toggle':'Search actions, chats, and settings.',
 'settings.toggle':'Open or leave Settings.',
 'settings.shortcuts':'Open this keyboard shortcuts page.',
 'approval.accept':'Approve the pending request when you are not typing.',
 'approval.reject':'Reject the pending request when you are not typing.',
};
export function KeyboardShortcutsSettings(){
 const bindings=useKeybindings();const [query,setQuery]=useState('');
 const [recording,setRecording]=useState<{command:MiraKeybindingCommand;index:number}|null>(null);
 const [notice,setNotice]=useState('');
 const rulesFor=(command:MiraKeybindingCommand):KeybindingRule[]=>bindings.filter(b=>b.command===command&&whenAstToExpression(b.whenAst)!=='false').map(b=>({command,key:shortcutToKeybindingInput(b.shortcut),when:whenAstToExpression(b.whenAst)||undefined}));
 const visible=MIRA_KEYBINDING_COMMANDS.filter(command=>!command.includes('.slot')).filter(command=>`${commandLabel(command)} ${descriptions[command]??''} ${bindings.filter(b=>b.command===command).map(b=>formatShortcutLabel(b.shortcut)).join(' ')}`.toLowerCase().includes(query.toLowerCase()));
 return <div className="space-y-2.5 text-foreground">
  <div className="flex items-center justify-between gap-4"><h2 className="flex min-h-7 items-center gap-2 text-[13px] font-medium text-muted-foreground"><Keyboard className="size-3.5"/>Keyboard shortcuts</h2><button type="button" className="inline-flex items-center gap-2 rounded-lg px-3 py-2 text-xs text-muted-foreground hover:bg-accent hover:text-foreground" onClick={()=>{MIRA_KEYBINDING_COMMANDS.forEach(resetKeybindingCommand);setRecording(null);setNotice('Default shortcuts restored.');}}><RotateCcw className="size-3.5"/>Reset all</button></div>
  <div className="relative"><Search aria-hidden className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"/><input type="search" aria-label="Search shortcuts" placeholder="Search shortcuts" value={query} onChange={e=>setQuery(e.target.value)} className="h-9 w-full rounded-md border border-border/60 bg-transparent pl-9 pr-4 text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring"/></div>
  <div className="flex flex-col gap-3 rounded-xl border border-border/60 bg-card/40 px-4 py-3 sm:flex-row sm:items-center sm:justify-between"><div><h3 className="text-[13.5px] font-medium">Number shortcuts</h3><p className="mt-0.5 text-[12px] text-muted-foreground/80">Switch among your nine most recent chats or open side panel tabs.</p></div><select aria-label="Number shortcut layout" value={rulesFor('panel.slot1')[0]?.key==='alt+1'?'chats':'tabs'} className="rounded-lg border border-border bg-background px-3 py-2 text-xs text-foreground" onChange={event=>{for(let i=1;i<=9;i++){for(const kind of ['chat','panel']){const command=`${kind}.slot${i}` as MiraKeybindingCommand;const modified=(event.target.value==='tabs')===(kind==='panel');setCommandKeybindings(command,[{command,key:`${modified?'mod+alt':'alt'}+${i}`,when:'!terminalFocus'}]);}}setNotice('Number shortcut layout saved.');}}><option value="tabs">{ /mac|iphone|ipad/i.test(navigator.platform)?'⌘⌥':'Ctrl+Alt+'}1–9 for tabs; Alt+1–9 for chats</option><option value="chats">{ /mac|iphone|ipad/i.test(navigator.platform)?'⌘⌥':'Ctrl+Alt+'}1–9 for chats; Alt+1–9 for tabs</option></select></div>
  <p className="text-xs text-muted-foreground">Click the pencil, then press your shortcut. Changes save automatically on this device. Escape cancels recording.</p>
  <div role="status" aria-live="polite" className="text-xs text-muted-foreground">{notice || null}</div>
  <div className="divide-y divide-border/50 overflow-hidden rounded-xl border border-border/60 bg-card/40 px-4">
  {visible.map(command=>{const rules=rulesFor(command);const shown=rules.length?rules:[null];return <div key={command} className="flex flex-col gap-3 py-3 sm:flex-row sm:items-center">
   <div className="min-w-0 flex-1"><div className="text-[13.5px] font-medium">{commandLabel(command).replace(/^(Chat|Terminal|Sidebar|Settings|Approval|Command Palette|Review): /,'')}</div><p className="mt-1 text-[12px] leading-relaxed text-muted-foreground/80">{descriptions[command]}</p></div>
   <div className="w-full space-y-2 sm:w-[44%]">{shown.map((rule,index)=>{const active=recording?.command===command&&recording.index===index;const binding=bindings.find(b=>b.command===command&&shortcutToKeybindingInput(b.shortcut)===rule?.key);return <div key={`${command}:${index}`} className="flex min-h-8 items-center gap-2">
    {active?<input data-keybinding-capture="" autoFocus readOnly aria-label={`Record shortcut for ${commandLabel(command)}`} placeholder="Press shortcut…" className="h-8 min-w-0 flex-1 rounded-md border border-ring bg-background px-2 text-xs outline-none" onKeyDown={event=>{if(event.nativeEvent.isComposing)return;if(event.key==='Tab'){setRecording(null);return;}event.preventDefault();event.stopPropagation();if(event.key==='Escape'){setRecording(null);return;}const key=keybindingFromKeyboardEvent(event.nativeEvent,navigator.platform);if(!key)return;const conflicts=bindings.filter(b=>b.command!==command&&shortcutConflictKey(b.shortcut)===shortcutConflictKey(parseKeybindingShortcut(key)!)&&whenAstToExpression(b.whenAst)!=='false');if(conflicts.length){setNotice(`Already used by ${commandLabel(conflicts[0].command)}. Choose another shortcut.`);return;}const next=[...rules];next[index]={command,key,when:rule?.when??DEFAULT_KEYBINDINGS.find(b=>b.command===command)?.when};setCommandKeybindings(command,next);setRecording(null);setNotice(`${commandLabel(command)} shortcut saved.`);}}/>:<><span className="min-w-20">{binding?<kbd className="inline-flex rounded-md bg-background/60 border border-border/60 px-2.5 py-1 text-[13px] font-medium leading-none text-muted-foreground">{formatShortcutLabel(binding.shortcut)}</kbd>:<span className="text-[13px] text-muted-foreground">Unassigned</span>}</span><button type="button" aria-label={`Edit shortcut for ${commandLabel(command)}`} className="rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground" onClick={()=>{setNotice('');setRecording({command,index});}}><Pencil className="size-3.5"/></button></>}
    {active?<button type="button" aria-label="Cancel recording" className="p-1.5 text-muted-foreground" onClick={()=>setRecording(null)}><X className="size-3.5"/></button>:rule?<button type="button" aria-label={`Remove ${binding?formatShortcutLabel(binding.shortcut):rule.key} for ${commandLabel(command)}`} className="ml-auto rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-destructive" onClick={()=>{setCommandKeybindings(command,rules.filter((_,i)=>i!==index));setNotice('Shortcut removed.');}}><Trash2 className="size-3.5"/></button>:null}
   </div>;})}
   <div className="flex gap-3 text-xs text-muted-foreground"><button type="button" aria-label={`Add shortcut for ${commandLabel(command)}`} className="inline-flex items-center gap-1 hover:text-foreground" onClick={()=>setRecording({command,index:rules.length})}><Plus className="size-3"/>Add</button><button type="button" className="hover:text-foreground" onClick={()=>{resetKeybindingCommand(command);setRecording(null);setNotice('Default restored.');}}>Reset</button></div>
   {recording?.command===command&&recording.index>=shown.length&&<input data-keybinding-capture="" autoFocus readOnly aria-label={`Additional shortcut for ${commandLabel(command)}`} placeholder="Press shortcut…" className="h-8 w-full rounded-md border border-ring bg-background px-2 text-xs" onKeyDown={event=>{if(event.nativeEvent.isComposing)return;if(event.key==='Tab'){setRecording(null);return;}event.preventDefault();event.stopPropagation();if(event.key==='Escape'){setRecording(null);return;}const key=keybindingFromKeyboardEvent(event.nativeEvent,navigator.platform);if(!key)return;if(bindings.some(b=>shortcutConflictKey(b.shortcut)===shortcutConflictKey(parseKeybindingShortcut(key)!)&&whenAstToExpression(b.whenAst)!=='false')){setNotice('That shortcut is already assigned. Choose another.');return;}setCommandKeybindings(command,[...rules,{command,key,when:DEFAULT_KEYBINDINGS.find(b=>b.command===command)?.when}]);setRecording(null);setNotice('Shortcut added.');}}/>}
   </div></div>;})}
  {!visible.length&&<p className="py-8 text-center text-sm text-muted-foreground">No shortcuts found.</p>}
  </div>
 </div>;
}
