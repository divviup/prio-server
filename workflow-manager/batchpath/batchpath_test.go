package batchpath

import (
	"reflect"
	"testing"
	"time"

	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"
)

func TestWithinInterval(t *testing.T) {
	bpl, err := NewList([]string{
		// Before interval
		"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
		// Exactly at interval start
		"kittens-seen/2020/10/31/20/30/202fe8e3-e63c-4170-8962-dac77b4cd640.batch",
		// within interval
		"kittens-seen/2020/10/31/20/35/0f0317b2-c612-48c2-b08d-d98529d6eae4.batch",
		"kittens-seen/2020/10/31/21/29/7a1c0fbc-2b7f-4307-8185-9ea88961bb64.batch",
		"kittens-seen/2020/10/31/21/35/af97ffdd-00fc-4d6a-9790-e5c0de82e7b0.batch",
		// Exactly at interval end
		"kittens-seen/2020/10/31/22/12/dc1dcb80-25a7-4e3f-9ff5-552b7d69e21a.batch",
		// Past interval
		"kittens-seen/2020/10/31/22/35/79f0a477-b65c-47c9-a2bf-a3b56c33824a.batch",
	})
	if err != nil {
		t.Fatalf("unexpected error %q", err)
	}

	intervalStart, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/30")
	intervalEnd, _ := time.Parse("2006/01/02/15/04", "2020/10/31/22/12")

	within := bpl.WithinInterval(wftime.Interval{
		Begin: intervalStart,
		End:   intervalEnd,
	})

	if !reflect.DeepEqual(within, []string{
		"kittens-seen/2020/10/31/20/30/202fe8e3-e63c-4170-8962-dac77b4cd640.batch",
		"kittens-seen/2020/10/31/20/35/0f0317b2-c612-48c2-b08d-d98529d6eae4.batch",
		"kittens-seen/2020/10/31/21/29/7a1c0fbc-2b7f-4307-8185-9ea88961bb64.batch",
		"kittens-seen/2020/10/31/21/35/af97ffdd-00fc-4d6a-9790-e5c0de82e7b0.batch",
	}) {
		t.Errorf("unexpected result %q", within)
	}
}
