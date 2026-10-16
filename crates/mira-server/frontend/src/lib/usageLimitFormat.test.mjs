import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
const load=async(file)=>{const out=ts.transpileModule(await readFile(new URL(file,import.meta.url),'utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText;return import(`data:text/javascript;base64,${Buffer.from(out).toString('base64')}`);};
const {windowLabel,resetsIn}=await load('./usageLimitFormat.ts');
const {usageProvider,providerKey,sortProviders}=await load('./usageProviders.ts');
test('window names read as people say them',()=>{
 assert.equal(windowLabel('five_hour'),'5-hour');
 assert.equal(windowLabel('seven_day'),'Weekly');
 assert.equal(windowLabel('seven_day_opus'),'Weekly · Opus');
 assert.equal(windowLabel('primary'),'Primary');
});
test('a countdown shows the two largest units and stops once it has passed',()=>{
 assert.equal(resetsIn(1000+2*3600+14*60,1000),'in 2h 14m');
 assert.equal(resetsIn(1000+3*86400+4*3600,1000),'in 3d 4h');
 assert.equal(resetsIn(1000+30,1000),'in 1m');
 assert.equal(resetsIn(999,1000),null);
 assert.equal(resetsIn(null,1000),null);
});
test('usage rows and agent drivers map to one provider each',()=>{
 assert.equal(providerKey(null),'mira');
 assert.equal(providerKey('claude-code'),'claude');
 assert.equal(usageProvider('claude').label,'Claude Code');
 assert.equal(usageProvider('claude-code').color,usageProvider('claude').color);
 assert.equal(usageProvider('some-agent').label,'Some Agent');
 assert.deepEqual(sortProviders(['zeta','codex','mira','claude']),['mira','claude','codex','zeta']);
});
const {windowSeconds,elapsedShare,paceOf}=await load('./usageLimitFormat.ts');
test('a window length comes from its name, and pace from use against time',()=>{
 assert.equal(windowSeconds('five_hour'),5*3600);
 assert.equal(windowSeconds('seven_day_opus'),7*86400);
 assert.equal(windowSeconds('5h'),5*3600);
 assert.equal(windowSeconds('primary'),null);
 // Two hours into a five-hour window: 40% elapsed.
 assert.equal(elapsedShare('five_hour',1000+3*3600,1000),0.4);
 assert.equal(paceOf(0.8,0.4),'ahead');
 assert.equal(paceOf(0.45,0.4),'on');
 assert.equal(paceOf(0.1,0.4),'behind');
 assert.equal(paceOf(0.5,null),null);
});
