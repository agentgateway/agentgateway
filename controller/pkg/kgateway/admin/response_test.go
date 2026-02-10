package admin_test

import (
	"errors"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/admin"
)

var _ = Describe("SnapshotResponseData", func() {
	// TODO(tim): these tests are brittle and coupled to K8s serialization internals. we should
	// refactor to test semantic equality (unmarshal and compare structs) rather than exact JSON
	// string matching since this is an internal only API.
	DescribeTable("MarshalJSONString",
		func(response admin.SnapshotResponseData, expectedString string) {
			responseStr := response.MarshalJSONString()
			Expect(responseStr).To(MatchJSON(expectedString))
		},
		Entry("successful response can be formatted as json",
			admin.SnapshotResponseData{
				Data:  "my data",
				Error: nil,
			},
			`{"data":"my data","error":""}`),
		Entry("errored response can be formatted as json",
			admin.SnapshotResponseData{
				Data:  "",
				Error: errors.New("one error"),
			},
			`{"data":"","error":"one error"}`),
		Entry("CR list can be formatted as json",
			admin.SnapshotResponseData{
				Data: []corev1.Namespace{
					{
						ObjectMeta: metav1.ObjectMeta{
							Name:      "name",
							Namespace: "namespace",
							ManagedFields: []metav1.ManagedFieldsEntry{{
								Manager: "manager",
							}},
						},
						TypeMeta: metav1.TypeMeta{
							Kind:       "kind",
							APIVersion: "version",
						},
					},
				},
				Error: nil,
			},
			`{"data":[{"kind":"kind","apiVersion":"version","metadata":{"name":"name","namespace":"namespace","managedFields":[{"manager":"manager"}]},"spec":{},"status":{}}],"error":""}`),
	)
})
