package shared

import apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"

// ObjectMetadata contains labels and annotations for metadata overlays.
type ObjectMetadata struct {
	// Map of string keys and values that can be used to organize and categorize
	// (scope and select) objects. May match selectors of replication controllers
	// and services.
	// More info: https://kubernetes.io/docs/concepts/overview/working-with-objects/labels
	// +optional
	Labels map[string]string `json:"labels,omitempty"`

	// Annotations is an unstructured key value map stored with a resource that may be
	// set by external tools to store and retrieve arbitrary metadata. They are not
	// queryable and should be preserved when modifying objects.
	// More info: https://kubernetes.io/docs/concepts/overview/working-with-objects/annotations
	// +optional
	Annotations map[string]string `json:"annotations,omitempty"`
}

// KubernetesResourceOverlay provides a mechanism to customize generated
// Kubernetes resources using [Strategic Merge
// Patch](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-api-machinery/strategic-merge-patch.md)
// semantics.
//
// # Overlay Application Order
//
// Overlays are applied **after** all typed configuration fields have been processed.
// The full merge order is:
//
//  1. GatewayClass typed configuration fields (e.g., replicas, image settings from parametersRef)
//  2. Gateway typed configuration fields (from infrastructure.parametersRef)
//  3. GatewayClass overlays are applied
//  4. Gateway overlays are applied
//
// This ordering means Gateway-level configuration overrides GatewayClass-level configuration
// at each stage. For example, if both levels set the same label, the Gateway value wins.
type KubernetesResourceOverlay struct {
	// metadata defines a subset of object metadata to be customized.
	// Labels and annotations are merged with existing values. If both GatewayClass
	// and Gateway parameters define the same label or annotation key, the Gateway
	// value takes precedence (applied second).
	// +optional
	Metadata *ObjectMetadata `json:"metadata,omitempty"`

	// Spec provides an opaque mechanism to configure the resource Spec.
	// This field accepts a complete or partial Kubernetes resource spec (e.g., PodSpec, ServiceSpec)
	// and will be merged with the generated configuration using **Strategic Merge Patch** semantics.
	//
	// # Application Order
	//
	// Overlays are applied after all typed configuration fields from both levels.
	// The full merge order is:
	//
	//  1. GatewayClass typed configuration fields
	//  2. Gateway typed configuration fields
	//  3. GatewayClass overlays
	//  4. Gateway overlays (can override all previous values)
	//
	// # Strategic Merge Patch & Deletion Guide
	//
	// This merge strategy allows you to override individual fields, merge lists, or delete items
	// without needing to provide the entire resource definition.
	//
	// **1. Replacing Values (Scalars):**
	// Simple fields (strings, integers, booleans) in your config will overwrite the generated defaults.
	//
	// **2. Merging Lists (Append/Merge):**
	// Lists with "merge keys" (like `containers` which merges on `name`, or `tolerations` which merges on `key`)
	// will append your items to the generated list, or update existing items if keys match.
	//
	// **3. Deleting Fields or List Items ($patch: delete):**
	// To remove a field or list item from the generated resource, use the
	// `$patch: delete` directive. This works for both map fields and list items,
	// and is the recommended approach because it works with both client-side
	// and server-side apply.
	//
	//	spec:
	//	  template:
	//	    spec:
	//	      # Delete pod-level securityContext
	//	      securityContext:
	//	        $patch: delete
	//	      # Delete nodeSelector
	//	      nodeSelector:
	//	        $patch: delete
	//	      containers:
	//	        # Be sure to use the correct proxy name here or you will add a container instead of modifying a container:
	//	        - name: proxy-name
	//	          # Delete container-level securityContext
	//	          securityContext:
	//	            $patch: delete
	//
	// **4. Null Values (server-side apply only):**
	// Setting a field to `null` can also remove it, but this ONLY works with
	// `kubectl apply --server-side` or equivalent. With regular client-side
	// `kubectl apply`, null values are stripped by kubectl before reaching
	// the API server, so the deletion won't occur. Prefer `$patch: delete`
	// for consistent behavior across both apply modes.
	//
	//	spec:
	//	  template:
	//	    spec:
	//	      nodeSelector: null  # Removes nodeSelector (server-side apply only!)
	//
	// **5. Replacing Maps Entirely ($patch: replace):**
	// To replace an entire map with your values (instead of merging), use `$patch: replace`.
	// This removes all existing keys and replaces them with only your specified keys.
	//
	//	spec:
	//	  template:
	//	    spec:
	//	      nodeSelector:
	//	        $patch: replace
	//	        custom-key: custom-value
	//
	// **6. Replacing Lists Entirely ($patch: replace):**
	// If you want to strictly define a list and ignore all generated defaults, use `$patch: replace`.
	//
	//	service:
	//	  spec:
	//	    ports:
	//	      - $patch: replace
	//	      - name: http
	//	        port: 80
	//	        targetPort: 8080
	//	        protocol: TCP
	//	      - name: https
	//	        port: 443
	//	        targetPort: 8443
	//	        protocol: TCP
	//
	// +optional
	// +kubebuilder:validation:Type=object
	// +kubebuilder:pruning:PreserveUnknownFields
	Spec *apiextensionsv1.JSON `json:"spec,omitempty"`
}
