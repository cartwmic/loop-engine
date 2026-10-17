"""Driver-owned accounting; no subprocess starts at import."""
import hashlib, json, os, signal, subprocess, sys, time
from pathlib import Path

ENGINE_SHA = "e1599f645f93d0b3a8d64403253f74e0d3e956145b80aa791f57cc3f752723bc"
LIMITS = {"jobs":2, "threads":2, "wall_seconds":3600, "processes":1200, "scratch_bytes":5*1024**3}
def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def inventory(root):
    return {str(p.relative_to(root)):sha(p) for p in sorted(Path(root).rglob("*")) if p.is_file()}
def digest(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(",",":")).encode()).hexdigest()
def save(path, value):
    path = Path(path)
    tmp = path.with_suffix(path.suffix+".writing")
    tmp.write_text(json.dumps(value,indent=2,ensure_ascii=False)+"\n")
    tmp.replace(path)
def need(condition, message):
    if not condition:
        raise RuntimeError(message)
def group_alive(pgid):
    try:
        os.killpg(pgid,0)
        return True
    except ProcessLookupError:
        return False
def admission(path, grant_id, initial=False):
    data = json.loads(Path(path).read_bytes())
    samples = data["samples"]
    need(len(samples)>=2,"two resource samples required")
    first, last = samples[-2:]
    need(last["at"]-first["at"]>=5,"resource samples less than five seconds apart")
    need(0<=time.time()-last["at"]<=30,"resource observation stale")
    for row in [first,last]:
        need(row["hold"] is False and row["pressure"]==1,"resource HOLD/pressure")
        need(row["page_headroom_bytes"]>=8*1024**3 and row["free_percent"]>=30,"memory admission")
        need(row["disk_free_bytes"]>=40*1024**3,"disk admission")
        need(row["active_allocations"]==[grant_id],"heavy overlap/allocation mismatch")
    need(last["swap_bytes"]<=first["swap_bytes"],"swap increased")
    return data

class ProcessRunner:
    def __init__(self, source, engine, output, backend=None, grant=None, tools=None):
        self.source=Path(source).resolve(strict=True)
        self.engine=Path(engine).resolve(strict=True)
        self.output=Path(output).absolute()
        need(not self.output.exists(),"fresh output required")
        need(not self.output.resolve().is_relative_to(self.source),"output inside source")
        self.backend=Path(backend).resolve(strict=True) if backend else None
        self.mode="scripted" if backend else "native"
        self.before=inventory(self.source)
        self.grant_path=Path(grant).resolve(strict=True) if grant else None
        self.start=time.monotonic()
        self.commands=[]; self.groups=[]; self.observations=[]; self.lease=None; self.last_scratch_check=0
        self.tools = tools or {}
        if not backend:
            need(set(self.tools) == {"python", "cargo", "rustc"}, "absolute tool bindings required")
            for path in self.tools.values():
                need(Path(path).is_absolute() and Path(path).resolve(strict=True) == Path(path), "tool must be an absolute resolved file")
            need(Path(sys.executable).resolve() == Path(self.tools["python"]), "interpreter differs from pinned binding")
            need(grant is not None,"native proof requires separate owner execution approval and allocation")
            self.grant=json.loads(self.grant_path.read_bytes())
            g=self.grant
            need(g["kind"]=="CALIBRATION_NEW_CURRENT_NATIVE_ALLOCATION" and g["execution_approved"] is True,
                 "not a native execution grant")
            need(g["state"]=="allocated" and g["source_sha256"]==digest(self.before),"grant source mismatch")
            need(g["engine_sha256"]==sha(self.engine)==ENGINE_SHA,"frozen engine mismatch")
            need(g["output_root"]==str(self.output),"grant output mismatch")
            need(g["limits"]==LIMITS,"grant limits differ")
            here=Path(__file__).resolve().parent
            files={p.name:sha(p) for p in [here/"new-current-proof.py",here/"proof_support.py",here/"assert-new-current-proof.py"]}
            need(g["harness_sha256"]==files,"grant harness mismatch")
            tool_bindings={k:{"path":str(p),"sha256":sha(p)} for k,p in self.tools.items()}
            need(g["tool_bindings"]==tool_bindings,"grant tool bindings mismatch")
            self.observations.append(admission(g["resource_file"],g["allocation_id"],True))
            self.lease=Path(str(self.grant_path)+".consumed")
            with self.lease.open("x") as f:
                json.dump({"driver_pid":os.getpid(),"allocation_id":g["allocation_id"],"output":str(self.output)},f)
        self.output.mkdir()
        (self.output/"commands").mkdir()
        self.env={k:v for k,v in os.environ.items() if not k.startswith(("LOOP_","XDG_","BOOKENDS_"))}
        self.env.update(CARGO_TARGET_DIR=str(self.output/"build"), CARGO_BUILD_JOBS="2", RUST_TEST_THREADS="2",
                        RAYON_NUM_THREADS="2", OMP_NUM_THREADS="2", PYTHONDONTWRITEBYTECODE="1",
                        XDG_DATA_HOME=str(self.output/"data"), XDG_STATE_HOME=str(self.output/"state"),
                        XDG_CONFIG_HOME=str(self.output/"xdg-config"), XDG_CACHE_HOME=str(self.output/"cache"))
        # HOME is inherited unchanged, never repurposed for isolation.
        # Artifacts/catalog/config/target/XDG locations are explicit task-owned paths.
        if self.tools:
            self.env["RUSTC"]=str(self.tools["rustc"])
        self.env.pop("RUSTC_WRAPPER",None)
        self.env.pop("RUSTC_WORKSPACE_WRAPPER",None)
        # Cargo reads the existing locked cache; no install/provisioning or shared target.
        self.env["CARGO_HOME"]=os.environ.get("CARGO_HOME",str(Path.home()/".cargo"))
        self.env["RUSTUP_HOME"]=os.environ.get("RUSTUP_HOME",str(Path.home()/".rustup"))
        self.check_limits()

    def check_limits(self):
        need(time.monotonic()-self.start<LIMITS["wall_seconds"],"wall-time ceiling")
        need(len(self.commands)<LIMITS["processes"],"process-count ceiling")
        self.check_scratch()
        if self.mode=="native":
            self.observations.append(admission(self.grant["resource_file"],self.grant["allocation_id"]))
        need(not any(group_alive(g) for g in self.groups),"owned residual process group before next call")

    def check_scratch(self, force=False):
        if force or time.monotonic()-self.last_scratch_check>=5:
            size=sum(p.stat().st_size for p in self.output.rglob("*") if p.is_file())
            need(size<=LIMITS["scratch_bytes"],"scratch ceiling")
            self.last_scratch_check=time.monotonic()

    def call(self, argv, packet=None, expected=0, timeout=30):
        self.check_limits()
        n=len(self.commands)+1
        directory=self.output/"commands"/("%04d"%n); directory.mkdir()
        argv=list(map(str,argv))
        payload="" if packet is None else json.dumps(packet,ensure_ascii=False)
        (directory/"stdin.txt").write_text(payload)
        actual=argv
        if self.backend:
            save(directory/"scripted-request.json",{"argv":argv,"stdin":payload,"source":str(self.source)})
            actual=[sys.executable,"-B",str(self.backend),str(directory/"scripted-request.json")]
        row={"index":n,"argv":argv,"actual_argv":actual,"backend":self.mode,"cwd":str(self.source),
             "expected_exit":expected,"timeout_seconds":timeout,"stdin_sha256":sha(directory/"stdin.txt"),
             "environment_limits":{k:self.env[k] for k in ["CARGO_BUILD_JOBS","RUST_TEST_THREADS","RAYON_NUM_THREADS","OMP_NUM_THREADS"]},
             "status":"starting"}
        self.commands.append(row)
        save(directory/"receipt.json",row)
        began=time.monotonic(); proc=None; cleanup=[]
        try:
            with (directory/"stdin.txt").open("rb") as inp, (directory/"stdout.txt").open("wb") as out, (directory/"stderr.txt").open("wb") as err:
                proc=subprocess.Popen(actual,cwd=self.source,env=self.env,stdin=inp,stdout=out,stderr=err,start_new_session=True)
                pgid=os.getpgid(proc.pid); self.groups.append(pgid)
                row.update(pid=proc.pid,pgid=pgid,status="running",started_at=time.time())
                save(directory/"receipt.json",row)
                while True:
                    if time.monotonic()-began>=timeout:
                        raise RuntimeError("command timeout")
                    if time.monotonic()-self.start>=LIMITS["wall_seconds"]:
                        raise RuntimeError("wall-time ceiling")
                    self.check_scratch()
                    if self.mode=="native":
                        admission(self.grant["resource_file"],self.grant["allocation_id"])
                    try:
                        code=proc.wait(timeout=.25)
                        break
                    except subprocess.TimeoutExpired:
                        continue
                row.update(exit_code=code,waited=True)
        except BaseException as error:
            row.update(status="failed",error=repr(error))
            raise
        finally:
            if proc:
                pgid=row["pgid"]
                if group_alive(pgid):
                    for sig in [signal.SIGTERM,signal.SIGKILL]:
                        if not group_alive(pgid): break
                        os.killpg(pgid,sig); cleanup.append(sig.name)
                        try: proc.wait(timeout=2)
                        except subprocess.TimeoutExpired: pass
                        # Parent supervises descendants; no unrelated processes targeted.
                        deadline=time.monotonic()+2
                        while group_alive(pgid) and time.monotonic()<deadline:
                            time.sleep(.02)
                row["waited"]=proc.poll() is not None
                row["exit_code"]=proc.returncode
                row["residual_group"]=group_alive(pgid)
            row.update(elapsed_seconds=time.monotonic()-began,cleanup_signals=cleanup)
            for name in ["stdout","stderr"]:
                p=directory/(name+".txt")
                if p.exists(): row[name+"_sha256"]=sha(p)
            save(directory/"receipt.json",row)
            save(self.output/"process-accounting.json",{"backend":self.mode,"commands":self.commands,"groups":self.groups,
                "residual_groups":[g for g in self.groups if group_alive(g)],"limits":LIMITS})
        need(not row.get("residual_group",True),"owned process group unreconciled")
        need(row.get("exit_code")==expected,"unexpected exit: "+json.dumps(row))
        row["status"]="completed"; save(directory/"receipt.json",row)
        return {"stdout":(directory/"stdout.txt").read_text(),"stderr":(directory/"stderr.txt").read_text(),"index":n}

    def finish(self, success):
        limit_error=None
        try: self.check_scratch(force=True)
        except RuntimeError as error: limit_error=repr(error)
        remaining=[g for g in self.groups if group_alive(g)]
        result={"backend":self.mode,"all_children_waited":all(r.get("waited") is True for r in self.commands),
                "residual_groups":remaining,"commands":self.commands,"source_after":inventory(self.source),
                "elapsed_seconds":time.monotonic()-self.start,"limits":LIMITS,
                "allocation_release":"not-allocated-scripted" if self.mode=="scripted" else "AWAIT_PARENT_RECONCILIATION_EXPLICIT_RELEASE",
                "limit_error":limit_error,"resource_observations":self.observations,
                "process_cap_scope":"direct recorded invocations; descendants are owned/reconciled by process group, not claimed exhaustively counted",
                "success":success and not remaining and limit_error is None}
        save(self.output/"final-accounting.json",result)
        return result
