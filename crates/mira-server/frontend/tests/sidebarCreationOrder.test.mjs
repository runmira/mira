import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { groupByCwd, dateSection }=await bundle(`export { groupByCwd, dateSection } from './src/components/sidebar/groups';`);
const chat=(id,cwd,created_at,updated_at=created_at,pinned=false)=>({id,cwd,created_at,updated_at,pinned});
const positions=groups=>groups.map(group=>[group.cwd,group.sessions.map(s=>s.id)]);
test('activity updates and switching the active folder preserve creation positions and explicit pins', () => {
  const chats=[chat('a-old','/a',10),chat('a-new','/a',30),chat('a-pin','/a',15,15,true),chat('b-old','/b',20)];
  const expected=[['/b',['b-old']],['/a',['a-pin','a-new','a-old']]];
  assert.deepEqual(positions(groupByCwd(chats,'/a')),expected);
  const reordered=[...chats].reverse().map(s=>({...s,updated_at:s.id==='a-old'?999999:1}));
  assert.deepEqual(positions(groupByCwd(reordered,'/b')),expected);
});
test('filtering and settled chats do not redefine a folder creation position', () => {
  const chats=[chat('a-old','/a',10),chat('a-new','/a',50),chat('b','/b',20)];
  const filtered=chats.filter(s=>s.id!=='a-old');
  assert.deepEqual(positions(groupByCwd(filtered,'/a',chats)),[['/b',['b']],['/a',['a-new']]]);
});
test('folder date headings use chat creation rather than latest activity', () => {
  const now=new Date(2026,9,14,12),created=new Date(2026,9,1,12).getTime()/1000;
  assert.equal(dateSection(chat('old','/a',created,now.getTime()/1000),now),'This month');
  assert.equal(dateSection(chat('unknown','/a',0,now.getTime()/1000),now),'Older');
});
