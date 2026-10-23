#!/usr/bin/env node
// Deterministic offline fixture generator. Requires Node.js 18+ only for regeneration.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const ROOT=path.dirname(fileURLToPath(import.meta.url));
const config=JSON.parse(fs.readFileSync(path.join(ROOT,'locale_config.json'),'utf8'));
function generateCases(config) {
 const cases=[];
 const wc=(s)=>s.trim().split(/\s+/u).filter(Boolean).length;
 const utf8len=(s)=>[...s].reduce((n,ch)=>{const c=ch.codePointAt(0);return n+(c<128?1:c<2048?2:c<65536?3:4);},0);
 const expand=(s,v)=>s.replace(/\{(\w+)\}/g,(_,k)=>v[k]===undefined?`{${k}}`:String(v[k]));
 const langs=Object.keys(config.profiles);
 for (let i=0;i<langs.length;i++){
  const lang=langs[i],l=config.profiles[lang];
  for(let t=0;t<10;t++){
   const vars={a:l.persons[t%5],b:l.others[(t+1)%5],city:l.cities[(t+2)%5],org:l.orgs[(t+3)%5],topic:l.topics[t],obj:l.objects[t],p1:41+(t%8),p2:15+(t%7),y1:2010+(t%4),y2:2012+(t%4),y3:2015+(t%4),y4:2018+(t%4),y5:2022+(t%4)};
   const listSize=t%2===0?10:6;
   const id=`${lang}-${String(t+1).padStart(2,"0")}-${config.domains[t]}`;
   const story=[`${l.name.toUpperCase()} | ${l.topics[t].toUpperCase()} | ${vars.city.toUpperCase()}`,
    expand(l.segments[0],vars),
    expand(l.timeline,vars),
    expand(l.quote,vars),
    expand(l.chart,vars),
    `${listSize===10?"Top 10":"Checklist"}:\n`+l.items.slice(0,listSize).map((v,j)=>`${j+1}. ${v}`).join("\n")];
   let step=0;
   while(wc(story.join("\n\n"))<1935 && step<40){
     const first=l.segments[(step*5+t+1)%6];
     const second=l.segments[(step*7+t+3)%6];
     // Deterministic permutation and recurring entities intentionally test co-mention and dedup.
     if(step>0&&step%5===0)story.push(`${l.topics[t].toUpperCase()} — ${vars.city.toUpperCase()} — ${step}`);
     story.push(expand(first,vars)+" "+expand(second,vars));
     step++;
   }
   story.push(expand(l.segments[5],vars));
   const body=story.join("\n\n")+"\n";
   const count=wc(body);
   if(count<1900||count>2160)throw Error(`word bound ${id} ${count}`);
   if(/\{\w+\}/.test(body))throw Error(`template unresolved ${id}`);
   cases.push({id,lang,language_name:l.name,topic:config.domains[t],path:`${lang}/${id}.txt`,text:body,
      word_count:count,byte_count:utf8len(body),
      expected_source_anchors:{person:vars.a,secondary_person:vars.b,city:vars.city,organization:vars.org,object:vars.obj,years:[vars.y1,vars.y2,vars.y3,vars.y4,vars.y5].map(String),percentages:[vars.p1,vars.p2],numbered_list_count:listSize}});
  }
 }
 return cases;
}
const all=generateCases(config);
for(const x of all){
 const filename=path.join(ROOT,x.path);
 fs.mkdirSync(path.dirname(filename),{recursive:true});
 fs.writeFileSync(filename,x.text,'utf8');
}
fs.writeFileSync(path.join(ROOT,'cases.json'),JSON.stringify(all.map(({text,...x})=>x),null,2)+'\n','utf8');
console.log(all.length+' documents, '+all.reduce((n,x)=>n+x.word_count,0)+' words');
