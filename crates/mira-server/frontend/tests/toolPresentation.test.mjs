import test from 'node:test';import assert from 'node:assert/strict';
import{toolPresentation}from'../src/lib/toolPresentation.mjs';
test('MCP outputs separate text and images',()=>{const o=toolPresentation(JSON.stringify({content:[{type:'text',text:'Clicked.'},{type:'image',mimeType:'image/png',data:'abc'}]}));assert.equal(o.text,'Clicked.');assert.deepEqual(o.images,[{media_type:'image/png',data:'abc'}]);});
test('empty results stay empty and arbitrary JSON is preserved',()=>{assert.equal(toolPresentation('{"content":[]}').text,'');assert.equal(toolPresentation('{"count":5}').text,'{"count":5}');assert.equal(toolPresentation('plain output').text,'plain output');});

test('MCP errors and unavailable images remain explicit',()=>{const result=toolPresentation(JSON.stringify({isError:true,content:[{type:'image',mimeType:'image/unknown',data:'secret-base64'}]}));assert.equal(result.isError,true);assert.equal(result.text,'Image unavailable.');assert.equal(result.images.length,0);});
