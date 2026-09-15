## End-to-end suite

This run was triggered with `run_e2e` on. After the local suite passes, also run the remote
end-to-end suite against the shared test environment.

- Run the local suite first. If it fails, stop and report — do not spend an environment slot on
  a change that is already broken.
- Quote the run identifier for the remote suite in your verdict so it can be found later.
- A remote failure that reproduces locally is a defect. One that does not is a flake: say so
  explicitly, say how many times you retried, and do not reject the change for it alone.
