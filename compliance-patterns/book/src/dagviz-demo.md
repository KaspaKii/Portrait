# Live BlockDAG Demo

<div style="position:relative;height:400px;border-radius:8px;overflow:hidden;background:#0f0f14;">
  <canvas id="dagviz-canvas" style="width:100%;height:100%;display:block;"></canvas>
</div>
<script type="module">
  import { KaspaDag } from '../../packages/kaspa-dagviz/dist/index.js';
  const canvas = document.getElementById('dagviz-canvas');
  if (canvas) {
    const dag = new KaspaDag({ theme: 'dark', visibleColumns: 7 });
    dag.mount(canvas);
  }
</script>

Live feed of the Kaspa mainnet blockDAG — 10 blocks per second, blue blocks highlighted by GHOSTDAG selection.

Built with `kaspa-dagviz` — zero dependencies, Canvas 2D, bundled Bézier edges.
