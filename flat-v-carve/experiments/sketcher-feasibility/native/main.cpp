#include "GCS.h"
#include "json.hpp"
#include <chrono>
#include <fstream>
#include <iostream>
#include <array>
using json=nlohmann::json;
using Clock=std::chrono::steady_clock;
double elapsed(Clock::time_point t) {return std::chrono::duration<double,std::milli>(Clock::now()-t).count();}
struct Model {
    std::vector<std::vector<double>> values;
    std::vector<double> dimensions;
    std::vector<std::array<double,4>> arcEnds;
    std::vector<GCS::Point> points;
    std::vector<GCS::Line> lines;
    std::vector<GCS::Circle> circles;
    std::vector<GCS::Arc> arcs;
    GCS::VEC_pD unknown;
    GCS::System system;
    Model(const json& f) {
        auto& es=f.at("entities"); auto& cs=f.at("constraints");
        size_t count=es.size(); values.resize(count);arcEnds.resize(count);points.resize(count);lines.resize(count);circles.resize(count);arcs.resize(count);dimensions.resize(cs.size());
        for(size_t i=0;i<count;i++) {
            auto& e=es[i]; values[i]=e.at("values").get<std::vector<double>>();auto& v=values[i];
            auto fix=e.value("fixed",std::vector<bool>{});
            for(size_t j=0;j<v.size();j++) if(j>=fix.size()||!fix[j]) unknown.push_back(&v[j]);
            std::string kind=e.at("kind"); points[i]=GCS::Point(&v[0],&v[1]);
            if(kind=="line") {lines[i].p1=points[i];lines[i].p2=GCS::Point(&v[2],&v[3]);}
            if(kind=="circle"||kind=="arc") {circles[i].center=points[i];circles[i].rad=&v[2];}
            if(kind=="arc") {
                auto& ar=arcs[i];auto& ends=arcEnds[i];
                ends={v[0]+v[2]*cos(v[3]),v[1]+v[2]*sin(v[3]),v[0]+v[2]*cos(v[4]),v[1]+v[2]*sin(v[4])};
                for(auto& x:ends) unknown.push_back(&x);
                ar.center=points[i];ar.rad=&v[2];ar.startAngle=&v[3];ar.endAngle=&v[4];ar.start=GCS::Point(&ends[0],&ends[1]);ar.end=GCS::Point(&ends[2],&ends[3]);
                system.addConstraintArcRules(ar,100000+static_cast<int>(i));
            }
        }
        for(size_t i=0;i<cs.size();i++) {
            auto& c=cs[i];std::string k=c.at("kind");int a=c.at("a"),b=c.value("b",0),tag=static_cast<int>(i)+1;
            dimensions[i]=c.value("value",0.);double* d=&dimensions[i];
            if(k=="horizontal")system.addConstraintHorizontal(lines[a],tag);
            else if(k=="vertical")system.addConstraintVertical(lines[a],tag);
            else if(k=="length")system.addConstraintP2PDistance(lines[a].p1,lines[a].p2,d,tag);
            else if(k=="radius")system.addConstraintCircleRadius(circles[a],d,tag);
            else if(k=="join")system.addConstraintP2PCoincident(lines[a].p2,lines[b].p1,tag);
            else if(k=="parallel")system.addConstraintParallel(lines[a],lines[b],tag);
            else if(k=="angle")system.addConstraintL2LAngle(lines[a],lines[b],d,tag);
            else if(k=="symmetry")system.addConstraintP2PSymmetric(points[a],points[b],lines[c.at("c").get<int>()],tag);
            else if(k=="perpendicular")system.addConstraintPerpendicular(lines[a],lines[b],tag);
            else if(k=="equal_length")system.addConstraintEqualLength(lines[a],lines[b],tag);
            else if(k=="equal_radius")system.addConstraintEqualRadius(circles[a],circles[b],tag);
            else if(k=="tangent")system.addConstraintTangent(lines[a],circles[b],tag);
            else if(k=="distance")system.addConstraintP2PDistance(points[a],points[b],d,tag);
            else if(k=="hdistance")system.addConstraintDifference(points[b].x,points[a].x,d,tag);
            else if(k=="vdistance")system.addConstraintDifference(points[b].y,points[a].y,d,tag);
            else if(k=="coincident")system.addConstraintP2PCoincident(points[a],points[b],tag);
            else if(k=="point_on_line")system.addConstraintPointOnLine(points[a],lines[b],tag);
            else if(k=="midpoint") {system.addConstraintPointOnLine(points[a],lines[b],tag);system.addConstraintPointOnPerpBisector(points[a],lines[b],tag);}
            else if(k=="point_on_circle")system.addConstraintPointOnCircle(points[a],circles[b],tag);
            else throw std::runtime_error("unsupported constraint "+k);
        }
        system.declareUnknowns(unknown);system.debugMode=GCS::NoDebug;
    }
    int solve() {system.initSolution();int status=system.solve();system.applySolution();return status;}
};
json run(const json& f) {
    auto start=Clock::now();Model m(f);double build=elapsed(start);
    start=Clock::now();int status=m.solve();double cold=elapsed(start);json values=m.values;
    start=Clock::now();m.system.diagnose();int dof=m.system.dofsNumber();GCS::VEC_I conflicts,redundant;m.system.getConflicting(conflicts);m.system.getRedundant(redundant);double diagnostic=elapsed(start);
    json times=json::array(),warm=json::array();
    if(f.contains("warm_constraint")) {
        int ci=f.at("warm_constraint"),samples=f.value("samples",1);double base=m.dimensions[ci],step=f.value("warm_step",0.01);
        for(int n=0;n<samples;n++) {double value=base+step*(n+1);m.dimensions[ci]=value;start=Clock::now();int s=m.solve();times.push_back(elapsed(start));warm.push_back({{"value",value},{"values",m.values},{"status",s}});}
    }
    json drag=json::array(); int released=-1;
    if(f.contains("drag")) {
        double x=0,y=0;
        m.system.addConstraintCoordinateX(m.lines[0].p2,&x,-1);
        m.system.addConstraintCoordinateY(m.lines[0].p2,&y,-1);
        for(const auto& cursor:f.at("drag")) {
            x=cursor[0];y=cursor[1];start=Clock::now();int s=m.solve();
            drag.push_back({{"cursor",cursor},{"ms",elapsed(start)},{"values",m.values},{"status",s}});
        }
        m.system.clearByTag(-1);m.system.initSolution();m.system.diagnose();released=m.system.dofsNumber();
    }
    return {{"id",f.at("id")},{"backend","planegcs"},{"status",status},{"values",values},{"dof",dof},{"conflicting",conflicts},{"redundant",redundant},{"build_ms",build},{"cold_ms",cold},{"diagnostic_ms",diagnostic},{"warm_ms",times},{"warm",warm},{"drag",drag},{"released_dof",released}};
}
int main(int argc,char**argv) {
    try {if(argc<3)throw std::runtime_error("usage: plane-probe fixtures.json output.json");std::ifstream input(argv[1]);json fixtures=json::parse(input),results=json::array();for(const auto& f:fixtures)results.push_back(run(f));std::ofstream(argv[2])<<results.dump();}
    catch(const std::exception& e){std::cerr<<e.what()<<"\n";return 1;}
}
