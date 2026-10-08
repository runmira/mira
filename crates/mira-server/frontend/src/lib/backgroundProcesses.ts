import {useCallback, useEffect, useRef, useState} from 'react';
import {sessionQuery} from './ndjson';
export interface BackgroundProcess {
  id:number; command:string; cwd:string; call_id:string; pid:number|null;
  running:boolean; exit_code:number|null; stopped:boolean; elapsed_secs:number;
  ports:number[]; url:string|null; last_line:string|null;
}
export async function stopBackgroundProcess(sessionId:string,id:number) {
  const response=await fetch(`/api/processes/${id}/stop${sessionQuery(sessionId)}`,{
    method:'POST',headers:{'content-type':'application/json'},body:'{}',
  });
  if (!response.ok) {
    const body=await response.json().catch(()=>({}));
    throw new Error(body.error ?? `Could not stop process (${response.status})`);
  }
  window.dispatchEvent(new CustomEvent('mira:processes-changed',{detail:sessionId}));
}
/** Poll only the visible chat, without scanning machine-wide ports. Requests
 * never overlap and old chat results cannot replace the new chat's processes. */
export function useBackgroundProcesses(sessionId:string,enabled=true) {
  const [snapshot,setSnapshot]=useState<{session:string;processes:BackgroundProcess[]}>({session:'',processes:[]});
  const [error,setError]=useState<string|null>(null);
  const [stopping,setStopping]=useState<ReadonlySet<string>>(new Set());
  const stopInFlight=useRef(new Set<string>());
  useEffect(()=>{
    setError(null);
    if (!sessionId || !enabled) return;
    const controller=new AbortController();let timer:ReturnType<typeof setTimeout>|undefined;let inFlight=false;
    const refresh=async()=>{
      if(controller.signal.aborted||document.hidden||inFlight)return;
      inFlight=true;clearTimeout(timer);
      try {
        const response=await fetch(`/api/processes${sessionQuery(sessionId)}&owned_only=true`,{signal:controller.signal});
        if(!response.ok)throw new Error(`Could not load background processes (${response.status})`);
        const data=await response.json() as {processes:BackgroundProcess[]};
        if(!controller.signal.aborted){setSnapshot({session:sessionId,processes:data.processes});setError(null);}
      } catch(error) {if(!controller.signal.aborted)setError((error as Error).message);}
      finally{inFlight=false;if(!controller.signal.aborted&&!document.hidden)timer=setTimeout(refresh,2000);}
    };
    const changed=(event:Event)=>{if((event as CustomEvent<string>).detail===sessionId)void refresh();};
    const visible=()=>{clearTimeout(timer);if(!document.hidden)void refresh();};
    window.addEventListener('mira:processes-changed',changed);document.addEventListener('visibilitychange',visible);
    void refresh();
    return()=>{controller.abort();clearTimeout(timer);window.removeEventListener('mira:processes-changed',changed);document.removeEventListener('visibilitychange',visible);};
  },[sessionId,enabled]);
  const stop=useCallback(async(id:number)=>{
    const key=`${sessionId}:${id}`;if(stopInFlight.current.has(key))return;
    stopInFlight.current.add(key);setStopping(new Set(stopInFlight.current));
    try{await stopBackgroundProcess(sessionId,id);
      setSnapshot(previous=>previous.session!==sessionId?previous:{...previous,processes:previous.processes.filter(process=>process.id!==id)});
    }finally{stopInFlight.current.delete(key);setStopping(new Set(stopInFlight.current));}
  },[sessionId]);
  return {processes:snapshot.session===sessionId?snapshot.processes:[],error,stop,stopping};
}
