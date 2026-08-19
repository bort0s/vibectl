#!/usr/bin/env node
'use strict';

/*
 * hook-overlap.js — did two hook invocations sharing a key run at the same time?
 *
 * ADR-0011 §7a's intra-agent-concurrency limit is decided by exactly that
 * question, and by nothing else: two invocations sharing (session_id, agent_id,
 * declared identity) that are ALIVE AT THE SAME TIME share a file, and no key
 * drawn from the payload can separate them.
 *
 * Reads the JSONL that `hook-collect.js` appends. Reports two instruments that
 * ask the same question independently:
 *
 *   1. INTERVALS — recorded [start_ms, end_ms] pairs intersect.
 *   2. CENSUS    — a marker file one invocation created was still on disk while
 *                  another was running. No clock is involved at all.
 *
 * WHY A ZERO NEEDS THE CROSS-KEY COUNT PRINTED BESIDE IT. ADR-0002 §7: a result
 * carries the proof its mechanism ran. If NEITHER instrument saw an overlap of
 * ANY kind, this run has not shown that either could see one, and the zero for
 * same-key pairs is a skipped test wearing a green tick. That case is called
 * out explicitly rather than left for a reader to notice. It has already
 * happened once: a collector recording only the microseconds around its own
 * work reported no overlap anywhere, and the zero was withheld.
 *
 * THE BOUND RUNS ONE WAY. `hook-collect.js` records a SUBSET of each process's
 * true lifetime, so an overlap reported here is real and a zero is evidence
 * rather than proof.
 *
 * usage: node hook-overlap.js <events.jsonl>
 */

const fs=require('fs');
const rows=[];
for(const l of fs.readFileSync(process.argv[2],'utf8').split('\n').filter(Boolean)){
  const r=JSON.parse(l); let p={}; try{p=JSON.parse(r.stdin);}catch(e){}
  rows.push({label:r.label, ev:p.hook_event_name, session:p.session_id, agent:p.agent_id||null,
             tool:p.tool_name||null, batch:(p.tool_calls||[]).map(t=>t.tool_name),
             pid:r.pid, s:r.start_ms, e:r.end_ms, d:r.dur_ms, marker:r.marker, census:r.census||[]});
}
const key=r=>(r.session||'?')+'|'+(r.agent||'<none>')+'|'+r.label;
console.log('invocations: '+rows.length+'   duration ms: min='+Math.min(...rows.map(r=>r.d)).toFixed(1)+' max='+Math.max(...rows.map(r=>r.d)).toFixed(1));
// instrument 1: intervals
let intra=[],cross=[];
for(let i=0;i<rows.length;i++)for(let j=i+1;j<rows.length;j++){
  const a=rows[i],b=rows[j];const ov=Math.min(a.e,b.e)-Math.max(a.s,b.s);
  if(ov<=0)continue;(key(a)===key(b)?intra:cross).push({a,b,ov});
}
console.log('INSTRUMENT 1 (intervals): overlapping pairs total='+(intra.length+cross.length)+'  same-key='+intra.length+'  different-key='+cross.length);
for(const p of intra.slice(0,12))console.log('   INTRA  '+p.a.ev+'/'+(p.a.tool||'-')+' pid'+p.a.pid+' x '+p.b.ev+'/'+(p.b.tool||'-')+' pid'+p.b.pid+'  overlap='+p.ov.toFixed(1)+'ms  agent='+(p.a.agent||'<none>')+' label='+p.a.label);
for(const p of cross.slice(0,8))console.log('   cross  '+p.a.ev+'['+p.a.label+',ag='+(p.a.agent||'-')+'] x '+p.b.ev+'['+p.b.label+',ag='+(p.b.agent||'-')+']  overlap='+p.ov.toFixed(1)+'ms');
// instrument 2: census
const byMarker=new Map(rows.map(r=>[r.marker,r]));
let cIntra=0,cCross=0,seen=new Set();
for(const r of rows) for(const m of r.census){
  if(m===r.marker) continue;
  const o=byMarker.get(m); const pk=[r.marker,m].sort().join('~');
  if(seen.has(pk)) continue; seen.add(pk);
  if(o && key(o)===key(r)){ cIntra++; console.log('   CENSUS INTRA  '+r.marker+'  saw  '+m+'   agent='+(r.agent||'<none>')); }
  else { cCross++; }
}
console.log('INSTRUMENT 2 (census): co-alive pairs total='+(cIntra+cCross)+'  same-key='+cIntra+'  different-key='+cCross);
if(intra.length+cross.length===0 && cIntra+cCross===0) console.log('>>> ZERO NOT REPORTABLE: neither instrument saw ANY overlap, so this run does not establish that either could.');
