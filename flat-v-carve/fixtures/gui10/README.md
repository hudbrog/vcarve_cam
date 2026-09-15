# GUI10 fixtures

`tight-holder.job.json` is the gui4 lettering carving with a tool assembly that
does not clear the job and an applied machine configuration whose holder is an
ER20 collet chuck. It exists so the browser scenario can exercise both machine
warnings through the shipped build rather than through a unit test:

* the endmill is a 2.5 mm cutter with a 2.5 mm shaft and a **0.8 mm stickout**,
  so the 25 mm nut stands 0.7 mm inside the material beside the carve;
* the machine names the `er20` holder, which is what gives the nut its diameter.

`create-fixture.mjs` writes it from the gui4 fixture; run it from the workspace
root after changing either the assembly numbers or the machine configuration:

```sh
node fixtures/gui10/create-fixture.mjs
```

Nothing here is machining truth: the job is a display fixture, and the checks it
exercises are display estimates over their own raster.
