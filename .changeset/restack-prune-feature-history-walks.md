---
"@treramey/graduate": patch
---

Speed up restack inspection on large repositories by pruning feature history walks at commits already reachable from main, loading only the commits the reconstruction proof reads, and caching decoded objects. Interactive restack failures now include the structured details that name the blocking commit or branches.
