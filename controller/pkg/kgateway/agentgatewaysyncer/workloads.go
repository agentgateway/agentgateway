package agentgatewaysyncer

import (
	"istio.io/istio/pilot/pkg/model"
	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/workloadapi"
)

func PrecomputeWorkload(w model.WorkloadInfo) model.WorkloadInfo {
	addr := WorkloadToAddress(w.Workload)
	setWorkloadAddress(&w, addr)
	return w
}

// setWorkloadAddress sets the AsAddress and MarshaledAddress fields on a WorkloadInfo
// using the provided Address. This is the shared logic for populating AsAddress.
func setWorkloadAddress(w *model.WorkloadInfo, addr *workloadapi.Address) {
	w.MarshaledAddress = protoconv.MessageToAny(addr)
	w.AsAddress = model.AddressInfo{
		Address:   addr,
		Marshaled: w.MarshaledAddress,
	}
}

// WorkloadToAddress converts a Workload to an Address.
// This is exported for potential use by downstream.
func WorkloadToAddress(w *workloadapi.Workload) *workloadapi.Address {
	return &workloadapi.Address{
		Type: &workloadapi.Address_Workload{
			Workload: w,
		},
	}
}
