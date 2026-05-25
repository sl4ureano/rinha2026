#ifndef DECISION_TREE_H
#define DECISION_TREE_H
#include <stdint.h>
#define TREE_LEAF 255
#define TREE_FEATURE_COUNT 21
typedef struct {
    uint8_t feature;
    float threshold;
    int16_t left, right;
    uint8_t fraud;
} tree_node_t;
extern const tree_node_t tree_nodes[];
extern const unsigned tree_node_count;
int tree_predict(const float features[TREE_FEATURE_COUNT]);
#endif

