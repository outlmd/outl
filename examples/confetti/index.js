"use strict";
(() => {
  // ../../plugin-sdk/src/index.ts
  function definePlugin(def) {
    if (def === null || typeof def !== "object") {
      throw new TypeError("definePlugin: expected a plugin definition object");
    }
    if (typeof def.activate !== "function") {
      throw new TypeError("definePlugin: `activate` must be a function");
    }
    if (def.deactivate !== void 0 && typeof def.deactivate !== "function") {
      throw new TypeError(
        "definePlugin: `deactivate` must be a function when provided"
      );
    }
    const host = globalThis;
    host.__outl_register?.(def);
    return def;
  }

  // src/index.ts
  var CONFETTI_HTML = `<!doctype html><html><head><meta charset="utf-8"><style>
  html,body{margin:0;height:100%;overflow:hidden;background:transparent}
  canvas{display:block}
</style></head><body><canvas id="c"></canvas><script>
  const cv=document.getElementById('c'),x=cv.getContext('2d');
  cv.width=innerWidth;cv.height=innerHeight;
  const colors=['#ff595e','#ffca3a','#8ac926','#1982c4','#6a4c93','#ff6ec7'];
  const P=[];
  for(let i=0;i<180;i++){P.push({
    x:innerWidth/2+(Math.random()-.5)*140, y:innerHeight*0.55,
    vx:(Math.random()-.5)*16, vy:Math.random()*-17-5,
    r:4+Math.random()*7, c:colors[i%colors.length], a:1,
    rot:Math.random()*6.28, vr:(Math.random()-.5)*.5
  });}
  let t=0;
  function frame(){
    x.clearRect(0,0,cv.width,cv.height); t++; let alive=false;
    for(const p of P){
      p.vy+=.45; p.x+=p.vx; p.y+=p.vy; p.rot+=p.vr; p.a-=.0075;
      if(p.a>0){
        alive=true; x.save(); x.globalAlpha=Math.max(0,p.a);
        x.translate(p.x,p.y); x.rotate(p.rot); x.fillStyle=p.c;
        x.fillRect(-p.r/2,-p.r/2,p.r,p.r*.6); x.restore();
      }
    }
    if(alive&&t<260) requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
</script></body></html>`;
  var index_default = definePlugin({
    activate(ctx) {
      ctx.ops.onOp((op) => {
        if (op.kind === "Edit" && op.todo === "DONE") {
          ctx.ui.render(CONFETTI_HTML);
        }
      });
    }
  });
})();
