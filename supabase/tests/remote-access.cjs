// Run from the repository root: node supabase/tests/remote-access.cjs
const ts = require(process.cwd() + '/crates/mira-server/frontend/node_modules/typescript');
const fs = require('fs'); const vm = require('vm'); const assert = require('assert/strict');
let source = fs.readFileSync('supabase/functions/remote-access/index.ts','utf8').replace(/import .*createClient.*;\n/, '');
source = ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
async function run({dbError=false, status=200}) {
 let handler, released=false;
 const query={select(){return this},eq(){return this},async maybeSingle(){return {data:{tunnel_id:'t',dns_record_id:'d'}}}};
 const db={auth:{getUser:async()=>({data:{user:{id:'u'}}})},from:()=>query,rpc:async()=>{released=true;return {error:dbError?{message:'delete failed'}:null}}};
 vm.runInNewContext(source,{createClient:()=>db,Deno:{env:{get:()=> 'test'},serve:f=>handler=f},fetch:async()=>new Response(JSON.stringify({success:status===200,result:{}}),{status}),Response,URL,console});
 const response=await handler(new Request('http://localhost/remote-access/disable',{method:'POST',headers:{Authorization:'Bearer test'},body:JSON.stringify({machine_id:'a'.repeat(32)})}));
 return {status:response.status,body:await response.json(),released};
}
(async()=>{
 assert.equal((await run({dbError:true})).status,500);
 assert.equal((await run({status:404})).status,200);
 const failed=await run({status:502});assert.equal(failed.status,502);assert.equal(failed.released,false);
 console.log('Disable: database failure reported, missing resources retry safely, DNS failure preserves rows');
 const ui=fs.readFileSync('crates/mira-server/frontend/src/components/settings/DevicesSection.tsx','utf8');
 const start=ui.slice(ui.indexOf('  async function startPairing()'),ui.indexOf('  async function revoke'));
 let resolveBaseline, opens=0, paired;
 const context={pairingStarting:{current:false},setPairingBusy:()=>{},setJustPaired:()=>{},remote:null,refresh:()=>new Promise(r=>resolveBaseline=r),openPairingCode:async()=>{opens++;return {code:'new',expires_at:100}},setPairing:v=>paired=v,setError:()=>{},Set};
 vm.runInNewContext(ts.transpileModule(start,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText,context);
 const first=context.startPairing();await context.startPairing();assert.equal(opens,0);
 resolveBaseline({devices:[{id:'existing'}]});await first;
 assert.equal(opens,1);assert(paired.known.has('existing'));assert.equal(context.pairingStarting.current,false);
 console.log('Pairing: repeated activation ignored, fresh baseline awaited and retained');
})().catch(e=>{console.error(e);process.exit(1)});
