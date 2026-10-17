//! Links `libstdcxx_compat.cpp` (see its header) into any binary that uses
//! the prebuilt sherpa-onnx static libs. Crates that link sherpa-onnx depend
//! on this one and `use nvr_sherpa_compat as _;` so it is never dropped.
