package batchpath

import (
	"fmt"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/letsencrypt/prio-server/workflow-manager/utils"
)

type BatchPath struct {
	AggregationID  string
	dateComponents []string
	ID             string
	Time           time.Time
	metadata       bool
	avro           bool
	sig            bool
}

type List []*BatchPath

func (bpl List) Len() int {
	return len(bpl)
}

func (bpl List) Less(i, j int) bool {
	return bpl[i].Time.Before(bpl[j].Time)
}

func (bpl List) Swap(i, j int) {
	bpl[i], bpl[j] = bpl[j], bpl[i]
}

func New(batchName string) (*BatchPath, error) {
	// batchName is like "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771"
	pathComponents := strings.Split(batchName, "/")
	batchID := pathComponents[len(pathComponents)-1]
	aggregationID := pathComponents[0]
	batchDate := pathComponents[1 : len(pathComponents)-1]

	if len(batchDate) != 5 {
		return nil, fmt.Errorf("malformed date in %q. Expected 5 date components, got %d", batchName, len(batchDate))
	}

	var dateComponents []int
	for _, c := range batchDate {
		parsed, err := strconv.ParseInt(c, 10, 64)
		if err != nil {
			return nil, fmt.Errorf("parsing date component %q in %q: %w", c, batchName, err)
		}
		dateComponents = append(dateComponents, int(parsed))
	}
	batchTime := time.Date(dateComponents[0], time.Month(dateComponents[1]),
		dateComponents[2], dateComponents[3], dateComponents[4], 0, 0, time.UTC)

	return &BatchPath{
		AggregationID:  aggregationID,
		dateComponents: batchDate,
		ID:             batchID,
		Time:           batchTime,
	}, nil
}

func (b *BatchPath) String() string {
	return fmt.Sprintf("{%s %s %s files:%d%d%d}", b.AggregationID, b.dateComponents, b.ID, utils.Index(!b.metadata), utils.Index(!b.avro), utils.Index(!b.sig))
}

func (b *BatchPath) path() string {
	return strings.Join([]string{b.AggregationID, b.DateString(), b.ID}, "/")
}

func (b *BatchPath) DateString() string {
	return strings.Join(b.dateComponents, "/")
}

func ReadyBatches(files []string, infix string) (List, error) {
	batches := make(map[string]*BatchPath)
	for _, name := range files {
		basename := basename(name, infix)
		b := batches[basename]
		var err error
		if b == nil {
			b, err = New(basename)
			if err != nil {
				return nil, err
			}
			batches[basename] = b
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s", infix)) {
			b.metadata = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s.avro", infix)) {
			b.avro = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s.sig", infix)) {
			b.sig = true
		}
	}

	var output []*BatchPath
	for _, v := range batches {
		output = append(output, v)
	}
	sort.Sort(List(output))

	return output, nil
}

// basename returns s, with any type suffixes stripped off. The type suffixes are determined by
// `infix`, which is one of "batch", "validity_0", or "validity_1".
func basename(s string, infix string) string {
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s", infix))
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s.avro", infix))
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s.sig", infix))
	return s
}
