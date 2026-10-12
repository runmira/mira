import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const browser = new Window({ url: 'http://localhost/' });
for (const key of ['window','document','navigator','localStorage','HTMLElement','HTMLInputElement','Element','Node','Event','CustomEvent','MutationObserver','NodeFilter','ResizeObserver','DocumentFragment','ShadowRoot','CSS','SVGElement','HTMLButtonElement']) Object.defineProperty(globalThis,key,{configurable:true,value:browser[key]});
globalThis.getComputedStyle=browser.getComputedStyle.bind(browser);
globalThis.requestAnimationFrame=browser.requestAnimationFrame.bind(browser);
globalThis.cancelAnimationFrame=browser.cancelAnimationFrame.bind(browser);
globalThis.IS_REACT_ACT_ENVIRONMENT=true;
document.write('<!doctype html><html><body></body></html>');
after(()=>browser.happyDOM.abort());
const {act}=await import('react');const {createRoot}=await import('react-dom/client');
const {render}=await bundle(`import { ComputerSection } from './src/components/settings/ComputerSection'; export const render=()=> <ComputerSection />;`);
const epoch='18446744073709551615';
const windowChoice={id:'window',epoch,title:'Editor',app:'Editor'};
const identity={name:'This computer',platform:'macOS',machine_id:'local',background_input:true,limitation:'',backend:'cua'};
const initial={enabled:false,target:'local',delivery:'background',window:windowChoice,computers:[],clients:[],can_change:true};
const response=(body,status=200)=>new Response(JSON.stringify(body),{status,headers:{'content-type':'application/json'}});
const settle=async()=>{await act(async()=>{await new Promise(r=>setTimeout(r,15));});};
async function mounted(fetcher){globalThis.fetch=fetcher;const host=document.createElement('div');document.body.append(host);const root=createRoot(host);await act(async()=>root.render(render()));await settle();return{host,root,close:async()=>{await act(async()=>root.unmount());host.remove();}};}
const button=(host,text)=>[...host.querySelectorAll('button')].find(b=>b.textContent.includes(text));
function reads(url){if(url==='/api/computer')return response(initial);if(url.endsWith('/status'))return response(identity);if(url.endsWith('/windows'))return response([windowChoice]);throw new Error(`Unexpected request ${url}`);}

test('saving access preserves the exact window handle, prevents duplicate mutations and reports failures',async()=>{
  let finish;let updates=[];
  const ui=await mounted(async(url,init={})=>{if(init.method==='PUT'){updates.push(JSON.parse(init.body));return new Promise(resolve=>{finish=resolve;});}return reads(url);});
  const toggle=ui.host.querySelector('[role="switch"]');assert.equal(toggle.disabled,false);
  await act(async()=>toggle.click());assert.equal(toggle.disabled,true);assert.equal(updates.length,1);assert.equal(updates[0].window.epoch,epoch);
  await act(async()=>finish(response({error:'Pause active chats before changing the computer.'},409)));
  assert.match(ui.host.querySelector('[role="alert"]').textContent,/Pause active chats/);assert.equal(toggle.getAttribute('aria-checked'),'false');
  await ui.close();
});

test('pairing inspects the machine before confirmation and does not enable computer use',async()=>{
  let inspections=0,connections=0;let view=initial;
  const ui=await mounted(async(url,init={})=>{
    if(url==='/api/computer')return response(view);
    if(url==='/api/computer/pairing/inspect'){inspections++;return response({...identity,name:'Studio Mac',machine_id:'remote'});}
    if(url==='/api/computer/pairing/connect'){connections++;const computer={id:'remote',name:'Studio Mac',platform:'macOS'};view={...initial,computers:[computer]};return response(computer);}
    return reads(url);
  });
  await act(async()=>button(ui.host,'Pair a computer').click());
  const input=document.querySelector('#computer-pairing-link');
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'https://machine.example/#computer-pair=AB12CD34EF');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>button(document.body,'Continue').click());await settle();assert.equal(inspections,1);assert.equal(connections,0);assert.match(document.body.textContent,/Confirm computer/);
  await act(async()=>button(document.body,'Pair computer').click());await settle();assert.equal(connections,1);assert.match(ui.host.textContent,/Studio Mac/);assert.equal(ui.host.querySelector('[role="switch"]').getAttribute('aria-checked'),'false');
  await ui.close();
});

test('closing a sharing sheet cancels only its own code even when opening resolves late',async()=>{
  let open;const closed=[];
  const ui=await mounted(async(url,init={})=>{
    if(url==='/api/remote')return response({enabled:true,status:'connected',port_stale:false,error:null});
    if(url==='/api/computer/pairing'&&init.method==='POST')return new Promise(resolve=>{open=resolve;});
    if(url==='/api/computer/pairing'&&init.method==='DELETE'){closed.push(JSON.parse(init.body).code);return new Response(null,{status:204});}
    return reads(url);
  });
  await act(async()=>button(ui.host,'Show pairing link').click());await settle();
  await act(async()=>button(document.body,'Done').click());
  await act(async()=>open(response({code:'AB12CD34EF',expires_at:Date.now()/1000+600,link:'https://machine.example/#computer-pair=AB12CD34EF'})));await settle();
  assert.deepEqual(closed,['AB12CD34EF']);await ui.close();
});
